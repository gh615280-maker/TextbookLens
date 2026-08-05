use std::{sync::Arc, time::Instant};

use parking_lot::Mutex;
use uuid::Uuid;

use crate::{
    domain::{
        ContentAnchor, DocumentLocator, LearningAction, LearningRequestEventPayload,
        LearningRequestStatus, MAX_LEARNING_OUTPUT_BYTES, SelectionAnchor, TextQuote,
    },
    errors::AppErrorCode,
};

use super::{
    EventSink, FollowupRequestContext, LEARNING_REQUEST_TERMINAL_TTL, LearningPersistenceTarget,
    LearningRequestContext, LearningRequestRegistry, MAX_ACTIVE_REQUESTS,
    MAX_RETAINED_TERMINAL_REQUESTS, NewSelectionRequestContext,
};

#[test]
fn registry_snapshot_subscription_is_monotonic_duplicate_safe_and_drops_dead_sinks() {
    let registry = LearningRequestRegistry::default();
    let created = registry.create(new_context()).expect("create request");
    assert_eq!(created.status, LearningRequestStatus::Preparing);
    assert_eq!(created.last_seq, 1);

    let observed = Arc::new(Mutex::new(Vec::new()));
    let first = collecting_sink(observed.clone());
    let snapshot = registry
        .subscribe(created.request_id, 0, 7, first)
        .expect("subscribe");
    assert_eq!(snapshot.last_seq, 1);
    registry
        .append_text(created.request_id, "visible ".to_owned())
        .expect("first delta");

    let replacement = collecting_sink(observed.clone());
    let replacement_snapshot = registry
        .subscribe(created.request_id, 1, 7, replacement)
        .expect("replace duplicate channel");
    assert_eq!(replacement_snapshot.text, "visible ");
    assert_eq!(replacement_snapshot.last_seq, 2);

    let dead: EventSink = Arc::new(|_| Err(()));
    registry
        .subscribe(created.request_id, 2, 8, dead)
        .expect("dead subscriber accepted initially");
    registry
        .record_usage(created.request_id, Some(10), Some(2))
        .expect("usage");
    registry
        .append_text(created.request_id, "answer".to_owned())
        .expect("second delta");

    let events = observed.lock();
    assert_eq!(events.len(), 3);
    assert_eq!(events[0].seq, 2);
    assert_eq!(events[1].seq, 3);
    assert_eq!(events[2].seq, 4);
    assert!(matches!(
        events[2].event,
        LearningRequestEventPayload::TextDelta { .. }
    ));
}

#[test]
fn registry_cancel_wins_until_database_commit_authorization_then_is_noop() {
    let registry = LearningRequestRegistry::default();
    let cancelled = registry.create(new_context()).expect("create cancelled");
    registry
        .append_text(cancelled.request_id, "partial".to_owned())
        .expect("delta");
    registry.cancel(cancelled.request_id).expect("cancel");
    registry
        .cancel(cancelled.request_id)
        .expect("idempotent cancel");
    assert!(
        !registry
            .begin_commit(cancelled.request_id)
            .expect("terminal begin is false")
    );
    assert_eq!(
        registry.snapshot(cancelled.request_id).unwrap().status,
        LearningRequestStatus::Cancelled
    );

    let before_database_commit = registry.create(new_context()).expect("create committing");
    registry
        .append_text(
            before_database_commit.request_id,
            "complete answer".to_owned(),
        )
        .expect("delta");
    assert!(
        registry
            .begin_commit(before_database_commit.request_id)
            .expect("begin commit")
    );
    registry
        .cancel(before_database_commit.request_id)
        .expect("stop before database commit wins");
    assert_eq!(
        registry
            .snapshot(before_database_commit.request_id)
            .unwrap()
            .status,
        LearningRequestStatus::Cancelled
    );

    let committed = registry.create(new_context()).expect("create committed");
    registry
        .append_text(committed.request_id, "complete answer".to_owned())
        .expect("delta");
    assert!(registry.begin_commit(committed.request_id).unwrap());
    assert!(
        registry
            .authorize_database_commit(committed.request_id)
            .expect("authorize exact database commit boundary")
    );
    registry
        .cancel(committed.request_id)
        .expect("stop after boundary is no-op");
    let conversation_id = Uuid::new_v4();
    registry
        .complete(committed.request_id, conversation_id)
        .expect("complete");
    let snapshot = registry.snapshot(committed.request_id).unwrap();
    assert_eq!(snapshot.status, LearningRequestStatus::Completed);
    assert_eq!(snapshot.conversation_id, Some(conversation_id));
}

#[test]
fn registry_enforces_utf8_byte_limit_before_append_and_redacts_debug() {
    let registry = LearningRequestRegistry::default();
    let created = registry.create(new_context()).expect("create");
    registry
        .append_text(created.request_id, "a".repeat(MAX_LEARNING_OUTPUT_BYTES))
        .expect("exact byte boundary");
    let error = registry
        .append_text(created.request_id, "界".to_owned())
        .expect_err("overflow rejected before append");
    assert_eq!(error.code, AppErrorCode::ContextTooLarge);
    let snapshot = registry.snapshot(created.request_id).unwrap();
    assert_eq!(snapshot.text.len(), MAX_LEARNING_OUTPUT_BYTES);
    assert_eq!(snapshot.status, LearningRequestStatus::Failed);

    let debug = format!(
        "{registry:?} {:?}",
        registry.commit_context(created.request_id)
    );
    assert!(!debug.contains("selected secret"));
    assert!(!debug.contains("model-secret"));
}

#[test]
fn registry_serializes_followups_per_conversation_but_not_across_conversations() {
    let registry = LearningRequestRegistry::default();
    let conversation_id = Uuid::new_v4();
    let first = registry
        .create(followup_context(conversation_id))
        .expect("first followup");
    let conflict = registry
        .create(followup_context(conversation_id))
        .expect_err("same conversation conflict");
    assert_eq!(conflict.code, AppErrorCode::RequestConflict);
    registry
        .create(followup_context(Uuid::new_v4()))
        .expect("other conversation independent");
    registry
        .cancel(first.request_id)
        .expect("release conversation");
    registry
        .create(followup_context(conversation_id))
        .expect("conversation reusable after terminal");
}

#[test]
fn registry_terminal_snapshot_survives_resubscribe_then_is_garbage_collected() {
    let registry = LearningRequestRegistry::default();
    let start = Instant::now();
    let created = registry
        .create_at_for_test(new_context(), start)
        .expect("create");
    registry.cancel(created.request_id).expect("cancel");
    registry
        .subscribe(created.request_id, 0, 1, Arc::new(|_| Ok(())))
        .expect("terminal resubscribe returns snapshot");
    registry.gc_at(start + LEARNING_REQUEST_TERMINAL_TTL + std::time::Duration::from_secs(1));
    assert_eq!(
        registry.snapshot(created.request_id).unwrap_err().code,
        AppErrorCode::NotFound
    );
}

#[test]
fn registry_terminal_retention_is_bounded_without_consuming_active_capacity() {
    let registry = LearningRequestRegistry::default();
    let mut terminal_ids = Vec::new();
    for _ in 0..(MAX_RETAINED_TERMINAL_REQUESTS + 8) {
        let created = registry
            .create(new_context())
            .expect("create terminal request");
        registry.cancel(created.request_id).expect("cancel request");
        terminal_ids.push(created.request_id);
    }

    let retained = terminal_ids
        .iter()
        .filter_map(|request_id| registry.snapshot(*request_id).ok())
        .collect::<Vec<_>>();
    assert_eq!(retained.len(), MAX_RETAINED_TERMINAL_REQUESTS);
    assert!(
        retained
            .iter()
            .all(|snapshot| snapshot.status == LearningRequestStatus::Cancelled)
    );

    let active = (0..MAX_ACTIVE_REQUESTS)
        .map(|_| {
            registry
                .create(new_context())
                .expect("terminal entries are not active")
        })
        .collect::<Vec<_>>();
    assert_eq!(active.len(), MAX_ACTIVE_REQUESTS);
    assert_eq!(
        registry.create(new_context()).unwrap_err().code,
        AppErrorCode::RequestConflict
    );
}

#[test]
fn registry_shutdown_cancels_and_clears_active_content_against_late_events() {
    let registry = LearningRequestRegistry::default();
    let created = registry.create(new_context()).unwrap();
    let cancel = registry.cancellation_token(created.request_id).unwrap();
    registry
        .append_text(created.request_id, "partial secret".to_owned())
        .unwrap();
    registry.shutdown();
    assert!(cancel.is_cancelled());
    assert_eq!(
        registry.snapshot(created.request_id).unwrap_err().code,
        AppErrorCode::NotFound
    );
    assert_eq!(
        registry
            .append_text(created.request_id, "late".to_owned())
            .unwrap_err()
            .code,
        AppErrorCode::NotFound
    );
}

fn collecting_sink(observed: Arc<Mutex<Vec<crate::domain::LearningRequestEvent>>>) -> EventSink {
    Arc::new(move |event| {
        observed.lock().push(event);
        Ok(())
    })
}

fn new_context() -> Arc<LearningRequestContext> {
    let section_id = Uuid::new_v4();
    Arc::new(LearningRequestContext {
        target: LearningPersistenceTarget::NewSelection(NewSelectionRequestContext {
            book_id: Uuid::new_v4(),
            section_id,
            anchor: ContentAnchor::Text {
                selection: SelectionAnchor {
                    locator: DocumentLocator::pdf(1, 1, None).unwrap(),
                    quote: TextQuote::new(
                        "selected secret".to_owned(),
                        String::new(),
                        String::new(),
                    )
                    .unwrap(),
                    section_id: Some(section_id),
                },
            },
            selected_text: Some("selected secret".to_owned()),
            action: LearningAction::Explain,
            question: "question secret".to_owned(),
        }),
        provider_profile_id: Uuid::new_v4(),
        model_id: "model-secret".to_owned(),
        available_citations: Vec::new(),
    })
}

fn followup_context(conversation_id: Uuid) -> Arc<LearningRequestContext> {
    Arc::new(LearningRequestContext {
        target: LearningPersistenceTarget::Followup(FollowupRequestContext {
            book_id: Uuid::new_v4(),
            conversation_id,
            expected_next_ordinal: 2,
            question: "followup secret".to_owned(),
        }),
        provider_profile_id: Uuid::new_v4(),
        model_id: "model-secret".to_owned(),
        available_citations: Vec::new(),
    })
}
