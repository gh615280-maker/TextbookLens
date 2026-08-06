use std::{path::Path, sync::Arc};

use futures_util::stream;
use sqlx::SqlitePool;
use textbooklens_lib::{
    ai::{error::AiError, provider::ProviderStream, registry::ProviderCapabilityRegistry},
    db::{
        Database,
        annotations::{MarkerRelocationStatus, list_annotation_markers},
        conversations::{
            DeleteSelectionConversation, LearningRepository, load_selection_conversation,
        },
    },
    domain::{
        ContentAnchor, DocumentLocator, LearningAction, LearningRequestStatus, SelectionAnchor,
        TextQuote, UnifiedStreamEvent,
    },
    errors::AppErrorCode,
    learning::{
        history::prepare_conversation_followup,
        orchestrator::LearningOrchestrator,
        registry::{
            LearningPersistenceTarget, LearningRequestContext, LearningRequestRegistry,
            NewSelectionRequestContext,
        },
    },
};
use uuid::Uuid;

const FIXTURE_TIME: &str = "2026-08-06T00:00:00.000Z";
const PARTIAL_SENTINEL: &str = "P12_INCOMPLETE_MEMORY_ONLY_SENTINEL";
const EVENT_SENTINEL: &str = "P12_EVENT_PRIVATE_CONTEXT_SENTINEL";
const MARKDOWN_SENTINEL: &str = "P12_MARKDOWN_PRIVATE_REASONING_SENTINEL";

#[test]
fn completed_history_and_marker_survive_real_sqlite_restart_but_partial_stream_does_not() {
    let temporary = tempfile::tempdir().unwrap();
    let database_path = temporary.path().join("learning-restart.sqlite3");
    let backup_path = temporary
        .path()
        .join("learning-restart-backup-fixture.sqlite3");
    let ids = FixtureIds::new();

    let database = Database::open(&database_path).unwrap();
    let (conversation_id, annotation_id) = tauri::async_runtime::block_on(async {
        seed(database.pool(), &ids).await;
        let registry = LearningRequestRegistry::default();
        let orchestrator = LearningOrchestrator::new(registry.clone(), database.pool().clone());

        let incomplete = registry
            .create(new_context(&ids, "incomplete selection", PARTIAL_SENTINEL))
            .unwrap();
        orchestrator
            .run_stream(
                incomplete.request_id,
                provider_stream(vec![UnifiedStreamEvent::TextDelta {
                    text: PARTIAL_SENTINEL.to_owned(),
                }]),
            )
            .await;
        let incomplete_snapshot = registry.snapshot(incomplete.request_id).unwrap();
        assert_eq!(incomplete_snapshot.status, LearningRequestStatus::Failed);
        assert_eq!(incomplete_snapshot.conversation_id, None);

        let completed = registry
            .create(new_context(
                &ids,
                "durable selection",
                "Explain the durable selection.",
            ))
            .unwrap();
        orchestrator
            .run_stream(
                completed.request_id,
                provider_stream(vec![
                    UnifiedStreamEvent::TextDelta {
                        text: "Durable restart answer.".to_owned(),
                    },
                    UnifiedStreamEvent::Completed,
                ]),
            )
            .await;
        let snapshot = registry.snapshot(completed.request_id).unwrap();
        assert_eq!(snapshot.status, LearningRequestStatus::Completed);
        let conversation_id = snapshot.conversation_id.unwrap();
        let annotation_id =
            sqlx::query_scalar::<_, String>("SELECT id FROM annotations WHERE conversation_id = ?")
                .bind(conversation_id.to_string())
                .fetch_one(database.pool())
                .await
                .unwrap();

        let event_json = serde_json::to_string(&snapshot).unwrap();
        for forbidden in [EVENT_SENTINEL, MARKDOWN_SENTINEL, PARTIAL_SENTINEL] {
            assert!(!event_json.contains(forbidden));
        }
        assert_counts(database.pool(), (1, 2, 1)).await;
        database.pool().close().await;
        (conversation_id, Uuid::parse_str(&annotation_id).unwrap())
    });
    drop(database);

    std::fs::copy(&database_path, &backup_path).unwrap();
    assert_backup_bytes_exclude(&backup_path, &[PARTIAL_SENTINEL, EVENT_SENTINEL]);

    let reopened = Database::open(&database_path).unwrap();
    tauri::async_runtime::block_on(async {
        let fresh_registry = LearningRequestRegistry::default();
        assert_eq!(
            fresh_registry.snapshot(conversation_id).unwrap_err().code,
            AppErrorCode::NotFound,
            "process-local requests and panel-open state are never restored"
        );
        let history = load_selection_conversation(reopened.pool(), ids.book, conversation_id)
            .await
            .unwrap();
        assert_eq!(history.annotation_id, annotation_id);
        assert_eq!(history.messages.len(), 2);
        assert_eq!(history.messages[1].content, "Durable restart answer.");
        assert_eq!(history.messages[1].provider_id, Some(ids.old_profile));
        assert_eq!(history.messages[1].model_id.as_deref(), Some("gpt-5.6"));

        let markers = list_annotation_markers(reopened.pool(), ids.book)
            .await
            .unwrap();
        assert_eq!(markers.len(), 1);
        assert_eq!(markers[0].id, annotation_id);
        assert_eq!(markers[0].conversation_id, Some(conversation_id));
        assert_eq!(
            markers[0].relocation_status,
            MarkerRelocationStatus::Primary
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM panel_preferences")
                .fetch_one(reopened.pool())
                .await
                .unwrap(),
            1
        );
        let forbidden_open_panel_tables =
            sqlx::query_scalar::<_, String>("SELECT name FROM sqlite_master WHERE type = 'table'")
                .fetch_all(reopened.pool())
                .await
                .unwrap()
                .into_iter()
                .filter(|name| name.contains("open_panel") || name.contains("panel_instance"))
                .collect::<Vec<_>>();
        assert!(forbidden_open_panel_tables.is_empty());
        assert_counts(reopened.pool(), (1, 2, 1)).await;
    });
    drop(reopened);
}

#[test]
fn restart_followup_captures_current_profile_preserves_history_and_delete_is_atomic() {
    let temporary = tempfile::tempdir().unwrap();
    let database_path = temporary.path().join("learning-followup-restart.sqlite3");
    let ids = FixtureIds::new();

    let database = Database::open(&database_path).unwrap();
    let (conversation_id, annotation_id) = tauri::async_runtime::block_on(async {
        seed(database.pool(), &ids).await;
        let registry = LearningRequestRegistry::default();
        let orchestrator = LearningOrchestrator::new(registry.clone(), database.pool().clone());
        let request = registry
            .create(new_context(
                &ids,
                "immutable restart selection",
                "Initial restart question.",
            ))
            .unwrap();
        orchestrator
            .run_stream(
                request.request_id,
                provider_stream(vec![
                    UnifiedStreamEvent::TextDelta {
                        text: "Initial historical answer.".to_owned(),
                    },
                    UnifiedStreamEvent::Completed,
                ]),
            )
            .await;
        let conversation_id = registry
            .snapshot(request.request_id)
            .unwrap()
            .conversation_id
            .unwrap();
        let annotation_id =
            sqlx::query_scalar::<_, String>("SELECT id FROM annotations WHERE conversation_id = ?")
                .bind(conversation_id.to_string())
                .fetch_one(database.pool())
                .await
                .unwrap();
        database.pool().close().await;
        (conversation_id, Uuid::parse_str(&annotation_id).unwrap())
    });
    drop(database);

    let reopened = Database::open(&database_path).unwrap();
    tauri::async_runtime::block_on(async {
        sqlx::query("UPDATE provider_profiles SET is_active = 0 WHERE id = ?")
            .bind(ids.old_profile.to_string())
            .execute(reopened.pool())
            .await
            .unwrap();
        sqlx::query("UPDATE provider_profiles SET is_active = 1 WHERE id = ?")
            .bind(ids.current_profile.to_string())
            .execute(reopened.pool())
            .await
            .unwrap();
        sqlx::query(
            "UPDATE app_settings SET active_provider_profile_id = ?, default_learning_profile_id = ? WHERE id = 1",
        )
        .bind(ids.current_profile.to_string())
        .bind(ids.current_profile.to_string())
        .execute(reopened.pool())
        .await
        .unwrap();

        let prepared = prepare_conversation_followup(
            reopened.pool(),
            &ProviderCapabilityRegistry::load_embedded().unwrap(),
            conversation_id,
            "Use the current profile after restart.".to_owned(),
        )
        .await
        .unwrap();
        assert_eq!(prepared.context.provider_profile_id, ids.current_profile);
        assert_eq!(prepared.context.model_id, "gemini-3.6-flash");
        let LearningPersistenceTarget::SelectionFollowup(target) = &prepared.context.target else {
            panic!("expected follow-up persistence target");
        };
        assert_eq!(target.book_id, ids.book);
        assert_eq!(target.conversation_id, conversation_id);
        assert_eq!(target.expected_next_ordinal, 2);

        let registry = LearningRequestRegistry::default();
        let orchestrator = LearningOrchestrator::new(registry.clone(), reopened.pool().clone());
        let followup = registry.create(prepared.context).unwrap();
        orchestrator
            .run_stream(
                followup.request_id,
                provider_stream(vec![
                    UnifiedStreamEvent::TextDelta {
                        text: "Current-profile answer.".to_owned(),
                    },
                    UnifiedStreamEvent::Completed,
                ]),
            )
            .await;
        let snapshot = registry.snapshot(followup.request_id).unwrap();
        assert_eq!(snapshot.status, LearningRequestStatus::Completed);
        assert_eq!(snapshot.conversation_id, Some(conversation_id));
        registry.cancel(followup.request_id).unwrap();
        assert_eq!(
            registry.snapshot(followup.request_id).unwrap().status,
            LearningRequestStatus::Completed,
            "Stop after the durable boundary is a no-op"
        );

        let history = load_selection_conversation(reopened.pool(), ids.book, conversation_id)
            .await
            .unwrap();
        assert_eq!(history.messages.len(), 4);
        assert_eq!(history.messages[1].provider_id, Some(ids.old_profile));
        assert_eq!(history.messages[1].model_id.as_deref(), Some("gpt-5.6"));
        assert_eq!(history.messages[3].provider_id, Some(ids.current_profile));
        assert_eq!(
            history.messages[3].model_id.as_deref(),
            Some("gemini-3.6-flash")
        );

        let _deletion = registry
            .begin_conversation_deletion(conversation_id)
            .unwrap();
        LearningRepository::new(reopened.pool().clone())
            .delete_selection_conversation(DeleteSelectionConversation {
                book_id: ids.book,
                conversation_id,
                annotation_id,
            })
            .await
            .unwrap();
        assert_counts(reopened.pool(), (0, 0, 0)).await;
        assert!(
            list_annotation_markers(reopened.pool(), ids.book)
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            load_selection_conversation(reopened.pool(), ids.book, conversation_id)
                .await
                .unwrap_err()
                .code,
            AppErrorCode::NotFound
        );
    });
    drop(reopened);
}

struct FixtureIds {
    book: Uuid,
    section: Uuid,
    old_profile: Uuid,
    current_profile: Uuid,
}

impl FixtureIds {
    fn new() -> Self {
        Self {
            book: Uuid::new_v4(),
            section: Uuid::new_v4(),
            old_profile: Uuid::new_v4(),
            current_profile: Uuid::new_v4(),
        }
    }
}

fn new_context(ids: &FixtureIds, selected: &str, question: &str) -> Arc<LearningRequestContext> {
    Arc::new(LearningRequestContext {
        target: LearningPersistenceTarget::NewSelection(NewSelectionRequestContext {
            book_id: ids.book,
            section_id: ids.section,
            anchor: text_anchor(ids.section, selected),
            selected_text: Some(selected.to_owned()),
            action: LearningAction::Explain,
            question: question.to_owned(),
        }),
        provider_profile_id: ids.old_profile,
        model_id: "gpt-5.6".to_owned(),
        available_citations: Vec::new(),
    })
}

fn provider_stream(events: Vec<UnifiedStreamEvent>) -> ProviderStream {
    Box::pin(stream::iter(
        events.into_iter().map(Ok::<UnifiedStreamEvent, AiError>),
    ))
}

fn text_anchor(section_id: Uuid, exact: &str) -> ContentAnchor {
    ContentAnchor::Text {
        selection: SelectionAnchor {
            locator: DocumentLocator::pdf(1, 1, None).unwrap(),
            quote: TextQuote::new(exact.to_owned(), String::new(), String::new()).unwrap(),
            section_id: Some(section_id),
        },
    }
}

async fn seed(pool: &SqlitePool, ids: &FixtureIds) {
    sqlx::query(
        "INSERT INTO books (id, sha256, title, format, original_filename, stored_path, import_status, created_at, updated_at) VALUES (?, ?, 'Synthetic restart book', 'pdf', 'synthetic.pdf', 'books/synthetic/original.pdf', 'ready', ?, ?)",
    )
    .bind(ids.book.to_string())
    .bind("d".repeat(64))
    .bind(FIXTURE_TIME)
    .bind(FIXTURE_TIME)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO sections (id, book_id, ordinal, title, locator_json) VALUES (?, ?, 0, 'Synthetic section', ?)",
    )
    .bind(ids.section.to_string())
    .bind(ids.book.to_string())
    .bind(serde_json::to_string(&DocumentLocator::pdf(1, 1, None).unwrap()).unwrap())
    .execute(pool)
    .await
    .unwrap();
    seed_profile(
        pool,
        ids.old_profile,
        "openai",
        "Historical profile",
        "gpt-5.6",
        true,
    )
    .await;
    seed_profile(
        pool,
        ids.current_profile,
        "gemini",
        "Current profile",
        "gemini-3.6-flash",
        false,
    )
    .await;
    sqlx::query(
        "UPDATE app_settings SET active_provider_profile_id = ?, default_learning_profile_id = ?, context_mode = 'standard' WHERE id = 1",
    )
    .bind(ids.old_profile.to_string())
    .bind(ids.old_profile.to_string())
    .execute(pool)
    .await
    .unwrap();
}

async fn seed_profile(
    pool: &SqlitePool,
    id: Uuid,
    kind: &str,
    name: &str,
    model: &str,
    active: bool,
) {
    sqlx::query(
        "INSERT INTO provider_profiles (id, provider_kind, display_name, model_id, context_window_tokens, is_active, created_at, updated_at, validated_at) VALUES (?, ?, ?, ?, 32000, ?, ?, ?, ?)",
    )
    .bind(id.to_string())
    .bind(kind)
    .bind(name)
    .bind(model)
    .bind(active)
    .bind(FIXTURE_TIME)
    .bind(FIXTURE_TIME)
    .bind(FIXTURE_TIME)
    .execute(pool)
    .await
    .unwrap();
}

async fn assert_counts(pool: &SqlitePool, expected: (i64, i64, i64)) {
    let conversations = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM conversations")
        .fetch_one(pool)
        .await
        .unwrap();
    let messages = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM messages")
        .fetch_one(pool)
        .await
        .unwrap();
    let annotations = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM annotations")
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!((conversations, messages, annotations), expected);
}

fn assert_backup_bytes_exclude(path: &Path, forbidden: &[&str]) {
    let bytes = std::fs::read(path).unwrap();
    for sentinel in forbidden {
        assert!(
            !bytes
                .windows(sentinel.len())
                .any(|window| window == sentinel.as_bytes()),
            "synthetic backup fixture contained forbidden sentinel"
        );
    }
}
