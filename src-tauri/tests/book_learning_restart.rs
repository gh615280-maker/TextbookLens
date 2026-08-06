use std::{path::Path, sync::Arc};

use futures_util::stream;
use sqlx::SqlitePool;
use textbooklens_lib::{
    ai::{error::AiError, provider::ProviderStream},
    db::{
        Database,
        annotations::list_annotation_markers,
        conversations::{DeleteBookConversation, LearningRepository, load_book_conversation},
    },
    domain::{DocumentLocator, LearningRequestStatus, UnifiedStreamEvent},
    errors::AppErrorCode,
    learning::{
        orchestrator::LearningOrchestrator,
        registry::{
            BookFollowupRequestContext, LearningPersistenceTarget, LearningRequestContext,
            LearningRequestRegistry, NewBookQuestionRequestContext,
        },
    },
};
use uuid::Uuid;

const FIXTURE_TIME: &str = "2026-08-06T00:00:00.000Z";
const PARTIAL_SENTINEL: &str = "P14_BOOK_PARTIAL_MEMORY_ONLY_SENTINEL";

#[test]
fn only_completed_book_history_survives_restart_and_delete_remains_atomic() {
    let temporary = tempfile::tempdir().unwrap();
    let database_path = temporary.path().join("book-learning-restart.sqlite3");
    let backup_path = temporary.path().join("book-learning-restart-copy.sqlite3");
    let book_id = Uuid::new_v4();
    let section_id = Uuid::new_v4();
    let old_profile = Uuid::new_v4();

    let database = Database::open(&database_path).unwrap();
    let conversation_id = tauri::async_runtime::block_on(async {
        seed_pdf(database.pool(), book_id, section_id).await;
        let registry = LearningRequestRegistry::default();
        let orchestrator = LearningOrchestrator::new(registry.clone(), database.pool().clone());

        let incomplete = registry
            .create(new_book_context(
                book_id,
                old_profile,
                "historical-book-model",
                "Incomplete book question?",
            ))
            .unwrap();
        orchestrator
            .run_stream(
                incomplete.request_id,
                provider_stream(vec![UnifiedStreamEvent::TextDelta {
                    text: PARTIAL_SENTINEL.to_owned(),
                }]),
            )
            .await;
        assert_eq!(
            registry.snapshot(incomplete.request_id).unwrap().status,
            LearningRequestStatus::Failed
        );

        let completed = registry
            .create(new_book_context(
                book_id,
                old_profile,
                "historical-book-model",
                "Durable book question?",
            ))
            .unwrap();
        orchestrator
            .run_stream(
                completed.request_id,
                provider_stream(vec![
                    UnifiedStreamEvent::TextDelta {
                        text: "Durable book answer.".to_owned(),
                    },
                    UnifiedStreamEvent::Completed,
                ]),
            )
            .await;
        let snapshot = registry.snapshot(completed.request_id).unwrap();
        assert_eq!(snapshot.status, LearningRequestStatus::Completed);
        assert_counts(database.pool(), (1, 2, 0)).await;
        let conversation_id = snapshot.conversation_id.unwrap();
        database.pool().close().await;
        conversation_id
    });
    drop(database);

    std::fs::copy(&database_path, &backup_path).unwrap();
    assert_file_excludes(&backup_path, PARTIAL_SENTINEL);

    let reopened = Database::open(&database_path).unwrap();
    tauri::async_runtime::block_on(async {
        let fresh_registry = LearningRequestRegistry::default();
        assert_eq!(
            fresh_registry.snapshot(conversation_id).unwrap_err().code,
            AppErrorCode::NotFound
        );
        let history = load_book_conversation(reopened.pool(), book_id, conversation_id)
            .await
            .unwrap();
        assert_eq!(history.messages.len(), 2);
        assert_eq!(history.messages[1].content, "Durable book answer.");
        assert_eq!(history.messages[1].provider_id, Some(old_profile));
        assert_eq!(
            history.messages[1].model_id.as_deref(),
            Some("historical-book-model")
        );
        assert!(
            list_annotation_markers(reopened.pool(), book_id)
                .await
                .unwrap()
                .is_empty()
        );

        let current_profile = Uuid::new_v4();
        let registry = LearningRequestRegistry::default();
        let orchestrator = LearningOrchestrator::new(registry.clone(), reopened.pool().clone());
        let followup = registry
            .create(book_followup_context(
                book_id,
                conversation_id,
                current_profile,
                "current-book-model",
                2,
            ))
            .unwrap();
        orchestrator
            .run_stream(
                followup.request_id,
                provider_stream(vec![
                    UnifiedStreamEvent::TextDelta {
                        text: "Current model follow-up.".to_owned(),
                    },
                    UnifiedStreamEvent::Completed,
                ]),
            )
            .await;
        assert_eq!(
            registry.snapshot(followup.request_id).unwrap().status,
            LearningRequestStatus::Completed
        );

        let history = load_book_conversation(reopened.pool(), book_id, conversation_id)
            .await
            .unwrap();
        assert_eq!(history.messages.len(), 4);
        assert_eq!(history.messages[1].provider_id, Some(old_profile));
        assert_eq!(
            history.messages[1].model_id.as_deref(),
            Some("historical-book-model")
        );
        assert_eq!(history.messages[3].provider_id, Some(current_profile));
        assert_eq!(
            history.messages[3].model_id.as_deref(),
            Some("current-book-model")
        );

        let deletion = registry
            .begin_conversation_deletion(conversation_id)
            .unwrap();
        LearningRepository::new(reopened.pool().clone())
            .delete_book_conversation(DeleteBookConversation {
                book_id,
                conversation_id,
            })
            .await
            .unwrap();
        assert_eq!(
            registry
                .create(book_followup_context(
                    book_id,
                    conversation_id,
                    Uuid::new_v4(),
                    "late-model",
                    4,
                ))
                .unwrap_err()
                .code,
            AppErrorCode::RequestConflict
        );
        drop(deletion);
        assert_counts(reopened.pool(), (0, 0, 0)).await;
        assert_eq!(
            load_book_conversation(reopened.pool(), book_id, conversation_id)
                .await
                .unwrap_err()
                .code,
            AppErrorCode::NotFound
        );

        let stale = registry
            .create(book_followup_context(
                book_id,
                conversation_id,
                Uuid::new_v4(),
                "stale-model",
                4,
            ))
            .unwrap();
        orchestrator
            .run_stream(
                stale.request_id,
                provider_stream(vec![
                    UnifiedStreamEvent::TextDelta {
                        text: "Late output cannot revive deletion.".to_owned(),
                    },
                    UnifiedStreamEvent::Completed,
                ]),
            )
            .await;
        assert_eq!(
            registry.snapshot(stale.request_id).unwrap().status,
            LearningRequestStatus::Failed
        );
        assert_counts(reopened.pool(), (0, 0, 0)).await;
    });
}

fn new_book_context(
    book_id: Uuid,
    profile_id: Uuid,
    model_id: &str,
    question: &str,
) -> Arc<LearningRequestContext> {
    Arc::new(LearningRequestContext {
        target: LearningPersistenceTarget::NewBookQuestion(NewBookQuestionRequestContext {
            book_id,
            question: question.to_owned(),
        }),
        provider_profile_id: profile_id,
        model_id: model_id.to_owned(),
        available_citations: Vec::new(),
    })
}

fn book_followup_context(
    book_id: Uuid,
    conversation_id: Uuid,
    profile_id: Uuid,
    model_id: &str,
    expected_next_ordinal: u32,
) -> Arc<LearningRequestContext> {
    Arc::new(LearningRequestContext {
        target: LearningPersistenceTarget::BookFollowup(BookFollowupRequestContext {
            book_id,
            conversation_id,
            expected_next_ordinal,
            question: "Continue the durable book discussion.".to_owned(),
        }),
        provider_profile_id: profile_id,
        model_id: model_id.to_owned(),
        available_citations: Vec::new(),
    })
}

fn provider_stream(events: Vec<UnifiedStreamEvent>) -> ProviderStream {
    Box::pin(stream::iter(
        events.into_iter().map(Ok::<UnifiedStreamEvent, AiError>),
    ))
}

async fn seed_pdf(pool: &SqlitePool, book_id: Uuid, section_id: Uuid) {
    sqlx::query(
        "INSERT INTO books (id, sha256, title, format, original_filename, stored_path, import_status, created_at, updated_at) VALUES (?, ?, 'Restart book', 'pdf', 'restart.pdf', 'books/restart/original.pdf', 'ready', ?, ?)",
    )
    .bind(book_id.to_string())
    .bind("d".repeat(64))
    .bind(FIXTURE_TIME)
    .bind(FIXTURE_TIME)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO sections (id, book_id, ordinal, title, locator_json) VALUES (?, ?, 0, 'Section', ?)",
    )
    .bind(section_id.to_string())
    .bind(book_id.to_string())
    .bind(serde_json::to_string(&DocumentLocator::pdf(1, 1, None).unwrap()).unwrap())
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

fn assert_file_excludes(path: &Path, forbidden: &str) {
    let bytes = std::fs::read(path).unwrap();
    assert!(
        !bytes
            .windows(forbidden.len())
            .any(|window| window == forbidden.as_bytes())
    );
}
