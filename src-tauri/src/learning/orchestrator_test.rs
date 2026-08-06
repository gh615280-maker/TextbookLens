use std::{sync::Arc, time::Instant};

use async_trait::async_trait;
use futures_util::stream;
use sqlx::SqlitePool;
use tokio::sync::{Mutex, oneshot};
use uuid::Uuid;

use crate::{
    ai::provider::ProviderStream,
    db::{
        Database,
        conversations::{
            LearningPersistenceFaultInjector, LearningPersistenceStep, LearningRepository,
        },
    },
    domain::{
        ContentAnchor, DocumentLocator, LearningAction, LearningRequestStatus, SelectionAnchor,
        TextQuote, UnifiedStreamEvent,
    },
    errors::{AppError, AppErrorCode, AppResult},
};

use super::{
    super::registry::{
        LEARNING_REQUEST_TERMINAL_TTL, LearningPersistenceTarget, LearningRequestContext,
        LearningRequestRegistry, NewBookQuestionRequestContext, NewSelectionRequestContext,
    },
    BoxLearningTask, LearningOrchestrator, LearningTaskSpawner,
};

const FIXTURE_TIME: &str = "2026-08-05T00:00:00.000Z";

#[tokio::test]
async fn orchestrator_persists_only_after_provider_completed() {
    let fixture = Fixture::new().await;
    let registry = LearningRequestRegistry::default();
    let orchestrator = LearningOrchestrator::new(registry.clone(), fixture.pool().clone());
    let created = registry.create(fixture.context()).unwrap();
    orchestrator
        .run_stream(
            created.request_id,
            provider_stream(vec![
                UnifiedStreamEvent::TextDelta {
                    text: "visible answer".to_owned(),
                },
                UnifiedStreamEvent::Usage {
                    input_tokens: Some(12),
                    output_tokens: Some(3),
                },
                UnifiedStreamEvent::Completed,
            ]),
        )
        .await;

    let snapshot = registry.snapshot(created.request_id).unwrap();
    assert_eq!(snapshot.status, LearningRequestStatus::Completed);
    assert_eq!(snapshot.text, "visible answer");
    assert_eq!(snapshot.usage.unwrap().output_tokens, Some(3));
    assert_counts(fixture.pool(), (1, 2, 1)).await;
}

#[tokio::test]
async fn orchestrator_gc_removes_only_process_local_output_and_keeps_durable_history() {
    let fixture = Fixture::new().await;
    let registry = LearningRequestRegistry::default();
    let orchestrator = LearningOrchestrator::new(registry.clone(), fixture.pool().clone());
    let created = registry.create(fixture.context()).unwrap();
    orchestrator
        .run_stream(
            created.request_id,
            provider_stream(vec![
                UnifiedStreamEvent::TextDelta {
                    text: "durable answer".to_owned(),
                },
                UnifiedStreamEvent::Completed,
            ]),
        )
        .await;

    registry
        .gc_at(Instant::now() + LEARNING_REQUEST_TERMINAL_TTL + std::time::Duration::from_secs(1));
    assert_eq!(
        registry.snapshot(created.request_id).unwrap_err().code,
        AppErrorCode::NotFound
    );
    assert_counts(fixture.pool(), (1, 2, 1)).await;
}

#[tokio::test]
async fn orchestrator_cancel_and_missing_terminal_never_persist_partial_streams() {
    let fixture = Fixture::new().await;
    let registry = LearningRequestRegistry::default();
    let orchestrator = LearningOrchestrator::new(registry.clone(), fixture.pool().clone());

    let cancelled = registry.create(fixture.context()).unwrap();
    registry.cancel(cancelled.request_id).unwrap();
    orchestrator
        .run_stream(
            cancelled.request_id,
            provider_stream(vec![
                UnifiedStreamEvent::TextDelta {
                    text: "late".to_owned(),
                },
                UnifiedStreamEvent::Completed,
            ]),
        )
        .await;
    assert_eq!(
        registry.snapshot(cancelled.request_id).unwrap().status,
        LearningRequestStatus::Cancelled
    );

    let incomplete = registry.create(fixture.context()).unwrap();
    orchestrator
        .run_stream(
            incomplete.request_id,
            provider_stream(vec![UnifiedStreamEvent::TextDelta {
                text: "partial only".to_owned(),
            }]),
        )
        .await;
    assert_eq!(
        registry.snapshot(incomplete.request_id).unwrap().status,
        LearningRequestStatus::Failed
    );
    assert_counts(fixture.pool(), (0, 0, 0)).await;
}

#[tokio::test]
async fn orchestrator_commit_fault_emits_failed_and_rolls_back_every_row() {
    let fixture = Fixture::new().await;
    let registry = LearningRequestRegistry::default();
    let repository =
        LearningRepository::with_fault_injector(fixture.pool().clone(), Arc::new(FailCommit));
    let orchestrator =
        LearningOrchestrator::with_repository(registry.clone(), fixture.pool().clone(), repository);
    let created = registry.create(fixture.context()).unwrap();
    orchestrator
        .run_stream(
            created.request_id,
            provider_stream(vec![
                UnifiedStreamEvent::TextDelta {
                    text: "complete provider output".to_owned(),
                },
                UnifiedStreamEvent::Completed,
            ]),
        )
        .await;
    let snapshot = registry.snapshot(created.request_id).unwrap();
    assert_eq!(snapshot.status, LearningRequestStatus::Failed);
    assert_eq!(snapshot.safe_error.unwrap().code, "DATABASE_ERROR");
    assert_counts(fixture.pool(), (0, 0, 0)).await;
}

#[tokio::test]
async fn orchestrator_stop_at_precommit_barrier_wins_and_rolls_back() {
    let fixture = Fixture::new().await;
    let registry = LearningRequestRegistry::default();
    let (reached_sender, reached_receiver) = oneshot::channel();
    let (release_sender, release_receiver) = oneshot::channel();
    let repository = LearningRepository::with_fault_injector(
        fixture.pool().clone(),
        Arc::new(CommitBarrier {
            reached: Mutex::new(Some(reached_sender)),
            release: Mutex::new(Some(release_receiver)),
        }),
    );
    let orchestrator =
        LearningOrchestrator::with_repository(registry.clone(), fixture.pool().clone(), repository);
    let created = registry.create(fixture.context()).unwrap();
    let request_id = created.request_id;
    let task = tokio::spawn(async move {
        orchestrator
            .run_stream(
                request_id,
                provider_stream(vec![
                    UnifiedStreamEvent::TextDelta {
                        text: "provider completed output".to_owned(),
                    },
                    UnifiedStreamEvent::Completed,
                ]),
            )
            .await;
    });
    reached_receiver.await.unwrap();
    registry.cancel(request_id).unwrap();
    release_sender.send(()).unwrap();
    task.await.unwrap();

    assert_eq!(
        registry.snapshot(request_id).unwrap().status,
        LearningRequestStatus::Cancelled
    );
    assert_counts(fixture.pool(), (0, 0, 0)).await;
}

#[tokio::test]
async fn book_orchestrator_stop_after_transaction_begins_but_before_authorizer_leaves_zero_rows() {
    let fixture = Fixture::new().await;
    let registry = LearningRequestRegistry::default();
    let (reached_sender, reached_receiver) = oneshot::channel();
    let (release_sender, release_receiver) = oneshot::channel();
    let repository = LearningRepository::with_fault_injector(
        fixture.pool().clone(),
        Arc::new(CommitBarrier {
            reached: Mutex::new(Some(reached_sender)),
            release: Mutex::new(Some(release_receiver)),
        }),
    );
    let orchestrator =
        LearningOrchestrator::with_repository(registry.clone(), fixture.pool().clone(), repository);
    let created = registry.create(fixture.book_context()).unwrap();
    let request_id = created.request_id;
    let task = tokio::spawn(async move {
        orchestrator
            .run_stream(
                request_id,
                provider_stream(vec![
                    UnifiedStreamEvent::TextDelta {
                        text: "provider completed book output".to_owned(),
                    },
                    UnifiedStreamEvent::Completed,
                ]),
            )
            .await;
    });
    reached_receiver.await.unwrap();
    registry.cancel(request_id).unwrap();
    release_sender.send(()).unwrap();
    task.await.unwrap();

    assert_eq!(
        registry.snapshot(request_id).unwrap().status,
        LearningRequestStatus::Cancelled
    );
    assert_counts(fixture.pool(), (0, 0, 0)).await;
}

#[tokio::test]
async fn orchestrator_spawn_failure_is_safe_terminal_without_marker_or_history() {
    let fixture = Fixture::new().await;
    let registry = LearningRequestRegistry::default();
    let orchestrator = LearningOrchestrator::new(registry.clone(), fixture.pool().clone());
    let created = registry.create(fixture.context()).unwrap();
    let snapshot = orchestrator
        .spawn_registered(created, Box::pin(async {}), &RejectSpawner)
        .unwrap();
    assert_eq!(snapshot.status, LearningRequestStatus::Failed);
    assert_eq!(snapshot.safe_error.unwrap().code, "PROVIDER_UNAVAILABLE");
    assert_counts(fixture.pool(), (0, 0, 0)).await;
}

struct RejectSpawner;

impl LearningTaskSpawner for RejectSpawner {
    fn spawn(&self, _task: BoxLearningTask) -> Result<(), ()> {
        Err(())
    }
}

struct FailCommit;

#[async_trait]
impl LearningPersistenceFaultInjector for FailCommit {
    async fn checkpoint(&self, step: LearningPersistenceStep) -> AppResult<()> {
        if step == LearningPersistenceStep::Commit {
            Err(AppError::new(AppErrorCode::DatabaseError))
        } else {
            Ok(())
        }
    }
}

struct CommitBarrier {
    reached: Mutex<Option<oneshot::Sender<()>>>,
    release: Mutex<Option<oneshot::Receiver<()>>>,
}

#[async_trait]
impl LearningPersistenceFaultInjector for CommitBarrier {
    async fn checkpoint(&self, step: LearningPersistenceStep) -> AppResult<()> {
        if step == LearningPersistenceStep::Commit {
            if let Some(sender) = self.reached.lock().await.take() {
                let _ = sender.send(());
            }
            if let Some(receiver) = self.release.lock().await.take() {
                let _ = receiver.await;
            }
        }
        Ok(())
    }
}

struct Fixture {
    _temporary: tempfile::TempDir,
    database: Database,
    book_id: Uuid,
    section_id: Uuid,
    profile_id: Uuid,
}

impl Fixture {
    async fn new() -> Self {
        let (temporary, database) = tokio::task::spawn_blocking(|| {
            let temporary = tempfile::tempdir().unwrap();
            let database = Database::open(temporary.path().join("orchestrator.sqlite3")).unwrap();
            (temporary, database)
        })
        .await
        .unwrap();
        let book_id = Uuid::new_v4();
        let section_id = Uuid::new_v4();
        seed_pdf(database.pool(), book_id, section_id).await;
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

    fn context(&self) -> Arc<LearningRequestContext> {
        Arc::new(LearningRequestContext {
            target: LearningPersistenceTarget::NewSelection(NewSelectionRequestContext {
                book_id: self.book_id,
                section_id: self.section_id,
                anchor: text_anchor(self.section_id, "immutable selection"),
                selected_text: Some("immutable selection".to_owned()),
                action: LearningAction::Explain,
                question: "Explain the selected textbook content.".to_owned(),
            }),
            provider_profile_id: self.profile_id,
            model_id: "captured-model".to_owned(),
            available_citations: Vec::new(),
        })
    }

    fn book_context(&self) -> Arc<LearningRequestContext> {
        Arc::new(LearningRequestContext {
            target: LearningPersistenceTarget::NewBookQuestion(NewBookQuestionRequestContext {
                book_id: self.book_id,
                question: "Ask the whole textbook.".to_owned(),
            }),
            provider_profile_id: self.profile_id,
            model_id: "captured-book-model".to_owned(),
            available_citations: Vec::new(),
        })
    }
}

fn provider_stream(events: Vec<UnifiedStreamEvent>) -> ProviderStream {
    Box::pin(stream::iter(events.into_iter().map(Ok)))
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
        "INSERT INTO books (id, sha256, title, format, original_filename, stored_path, import_status, created_at, updated_at) VALUES (?, ?, 'Synthetic book', 'pdf', 'synthetic.pdf', 'books/synthetic/original.pdf', 'ready', ?, ?)",
    )
    .bind(book_id.to_string())
    .bind("a".repeat(64))
    .bind(FIXTURE_TIME)
    .bind(FIXTURE_TIME)
    .execute(pool)
    .await
    .unwrap();
    let locator = serde_json::to_string(&DocumentLocator::pdf(1, 1, None).unwrap()).unwrap();
    sqlx::query(
        "INSERT INTO sections (id, book_id, ordinal, title, locator_json) VALUES (?, ?, 0, 'Section', ?)",
    )
    .bind(section_id.to_string())
    .bind(book_id.to_string())
    .bind(locator)
    .execute(pool)
    .await
    .unwrap();
}

async fn assert_counts(pool: &SqlitePool, expected: (i64, i64, i64)) {
    let conversations: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM conversations")
        .fetch_one(pool)
        .await
        .unwrap();
    let messages: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages")
        .fetch_one(pool)
        .await
        .unwrap();
    let annotations: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM annotations")
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!((conversations, messages, annotations), expected);
}
