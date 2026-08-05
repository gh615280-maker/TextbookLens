use std::sync::{Arc, Mutex};

use futures_util::stream;
use sqlx::SqlitePool;
use textbooklens_lib::{
    ai::{error::AiError, provider::ProviderStream},
    db::Database,
    domain::{
        ContentAnchor, DocumentLocator, LearningAction, LearningRequestEvent,
        LearningRequestEventPayload, LearningRequestStatus, MAX_LEARNING_OUTPUT_BYTES,
        SelectionAnchor, TextQuote, UnifiedStreamEvent,
    },
    errors::AppErrorCode,
    learning::{
        orchestrator::LearningOrchestrator,
        registry::{
            FollowupRequestContext, LearningPersistenceTarget, LearningRequestContext,
            LearningRequestRegistry, NewSelectionRequestContext,
        },
    },
};
use uuid::Uuid;

const FIXTURE_TIME: &str = "2026-08-05T00:00:00.000Z";

#[test]
fn concurrent_requests_are_independent_and_only_completed_stream_persists() {
    let fixture = Fixture::new();
    tauri::async_runtime::block_on(async {
        let registry = LearningRequestRegistry::default();
        let orchestrator = LearningOrchestrator::new(registry.clone(), fixture.pool().clone());
        let stopped = registry
            .create(fixture.new_context("stopped selection"))
            .unwrap();
        let completed = registry
            .create(fixture.new_context("completed selection"))
            .unwrap();
        let events = Arc::new(Mutex::new(Vec::<LearningRequestEvent>::new()));
        let observed = events.clone();
        registry
            .subscribe(
                completed.request_id,
                completed.last_seq,
                11,
                Arc::new(move |event| {
                    observed.lock().unwrap().push(event);
                    Ok(())
                }),
            )
            .unwrap();

        registry.cancel(stopped.request_id).unwrap();
        orchestrator
            .run_stream(
                stopped.request_id,
                provider_stream(vec![
                    UnifiedStreamEvent::TextDelta {
                        text: "late partial".to_owned(),
                    },
                    UnifiedStreamEvent::Completed,
                ]),
            )
            .await;
        orchestrator
            .run_stream(
                completed.request_id,
                provider_stream(vec![
                    UnifiedStreamEvent::TextDelta {
                        text: "durable answer".to_owned(),
                    },
                    UnifiedStreamEvent::Usage {
                        input_tokens: Some(30),
                        output_tokens: Some(4),
                    },
                    UnifiedStreamEvent::Completed,
                ]),
            )
            .await;

        assert_eq!(
            registry.snapshot(stopped.request_id).unwrap().status,
            LearningRequestStatus::Cancelled
        );
        let completed_snapshot = registry.snapshot(completed.request_id).unwrap();
        assert_eq!(completed_snapshot.status, LearningRequestStatus::Completed);
        assert_eq!(completed_snapshot.text, "durable answer");
        registry
            .cancel(completed.request_id)
            .expect("stop after durable commit is a no-op");
        assert_eq!(
            registry.snapshot(completed.request_id).unwrap().status,
            LearningRequestStatus::Completed
        );
        assert_counts(fixture.pool(), (1, 2, 1)).await;
        let observed = events.lock().unwrap();
        assert_eq!(observed.len(), 3);
        assert!(observed.windows(2).all(|pair| pair[0].seq < pair[1].seq));
        assert!(matches!(
            observed.last().unwrap().event,
            LearningRequestEventPayload::Completed { .. }
        ));
    });
}

#[test]
fn followup_is_single_flight_per_conversation_and_appends_one_atomic_pair() {
    let fixture = Fixture::new();
    tauri::async_runtime::block_on(async {
        let registry = LearningRequestRegistry::default();
        let orchestrator = LearningOrchestrator::new(registry.clone(), fixture.pool().clone());
        let initial = registry
            .create(fixture.new_context("initial selection"))
            .unwrap();
        orchestrator
            .run_stream(
                initial.request_id,
                provider_stream(vec![
                    UnifiedStreamEvent::TextDelta {
                        text: "initial answer".to_owned(),
                    },
                    UnifiedStreamEvent::Completed,
                ]),
            )
            .await;
        let conversation_id = registry
            .snapshot(initial.request_id)
            .unwrap()
            .conversation_id
            .unwrap();

        let followup = registry
            .create(fixture.followup_context(conversation_id, 2, "Follow up?"))
            .unwrap();
        let conflict = registry
            .create(fixture.followup_context(conversation_id, 2, "Decoy?"))
            .unwrap_err();
        assert_eq!(conflict.code, AppErrorCode::RequestConflict);
        registry
            .create(fixture.followup_context(Uuid::new_v4(), 2, "Independent?"))
            .expect("other conversation can be active");

        orchestrator
            .run_stream(
                followup.request_id,
                provider_stream(vec![
                    UnifiedStreamEvent::TextDelta {
                        text: "followup answer".to_owned(),
                    },
                    UnifiedStreamEvent::Completed,
                ]),
            )
            .await;
        let snapshot = registry.snapshot(followup.request_id).unwrap();
        assert_eq!(snapshot.status, LearningRequestStatus::Completed);
        assert_eq!(snapshot.conversation_id, Some(conversation_id));
        assert_counts(fixture.pool(), (1, 4, 1)).await;
        let assistant_models = sqlx::query_scalar::<_, String>(
            "SELECT model_id FROM messages WHERE conversation_id = ? AND role = 'assistant' ORDER BY ordinal",
        )
        .bind(conversation_id.to_string())
        .fetch_all(fixture.pool())
        .await
        .unwrap();
        assert_eq!(
            assistant_models,
            ["captured-model", "captured-followup-model"]
        );
    });
}

#[test]
fn utf8_output_overflow_cancels_provider_and_leaves_database_empty() {
    let fixture = Fixture::new();
    tauri::async_runtime::block_on(async {
        let registry = LearningRequestRegistry::default();
        let orchestrator = LearningOrchestrator::new(registry.clone(), fixture.pool().clone());
        let created = registry
            .create(fixture.new_context("overflow selection"))
            .unwrap();
        let cancel = registry.cancellation_token(created.request_id).unwrap();
        orchestrator
            .run_stream(
                created.request_id,
                provider_stream(vec![
                    UnifiedStreamEvent::TextDelta {
                        text: "a".repeat(MAX_LEARNING_OUTPUT_BYTES),
                    },
                    UnifiedStreamEvent::TextDelta {
                        text: "界".to_owned(),
                    },
                    UnifiedStreamEvent::Completed,
                ]),
            )
            .await;
        let snapshot = registry.snapshot(created.request_id).unwrap();
        assert_eq!(snapshot.status, LearningRequestStatus::Failed);
        assert_eq!(snapshot.text.len(), MAX_LEARNING_OUTPUT_BYTES);
        assert_eq!(snapshot.safe_error.unwrap().code, "CONTEXT_TOO_LARGE");
        assert!(cancel.is_cancelled());
        assert_counts(fixture.pool(), (0, 0, 0)).await;
    });
}

struct Fixture {
    _temporary: tempfile::TempDir,
    database: Database,
    book_id: Uuid,
    section_id: Uuid,
    profile_id: Uuid,
}

impl Fixture {
    fn new() -> Self {
        let temporary = tempfile::tempdir().unwrap();
        let database = Database::open(temporary.path().join("stream-lifecycle.sqlite3")).unwrap();
        let book_id = Uuid::new_v4();
        let section_id = Uuid::new_v4();
        tauri::async_runtime::block_on(seed_pdf(database.pool(), book_id, section_id));
        Self {
            _temporary: temporary,
            database,
            book_id,
            section_id,
            profile_id: Uuid::new_v4(),
        }
    }

    fn pool(&self) -> &SqlitePool {
        self.database.pool()
    }

    fn new_context(&self, selected: &str) -> Arc<LearningRequestContext> {
        Arc::new(LearningRequestContext {
            target: LearningPersistenceTarget::NewSelection(NewSelectionRequestContext {
                book_id: self.book_id,
                section_id: self.section_id,
                anchor: text_anchor(self.section_id, selected),
                selected_text: Some(selected.to_owned()),
                action: LearningAction::Explain,
                question: "Explain the selected textbook content.".to_owned(),
            }),
            provider_profile_id: self.profile_id,
            model_id: "captured-model".to_owned(),
            available_citations: Vec::new(),
        })
    }

    fn followup_context(
        &self,
        conversation_id: Uuid,
        expected_next_ordinal: u32,
        question: &str,
    ) -> Arc<LearningRequestContext> {
        Arc::new(LearningRequestContext {
            target: LearningPersistenceTarget::Followup(FollowupRequestContext {
                book_id: self.book_id,
                conversation_id,
                expected_next_ordinal,
                question: question.to_owned(),
            }),
            provider_profile_id: self.profile_id,
            model_id: "captured-followup-model".to_owned(),
            available_citations: Vec::new(),
        })
    }
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

async fn seed_pdf(pool: &SqlitePool, book_id: Uuid, section_id: Uuid) {
    sqlx::query(
        "INSERT INTO books (id, sha256, title, format, original_filename, stored_path, import_status, created_at, updated_at) VALUES (?, ?, 'Lifecycle book', 'pdf', 'lifecycle.pdf', 'books/lifecycle/original.pdf', 'ready', ?, ?)",
    )
    .bind(book_id.to_string())
    .bind("c".repeat(64))
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
