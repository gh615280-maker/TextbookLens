use std::time::Duration;
use std::{fs, sync::Arc};

use tokio::{sync::oneshot, task::yield_now, time::timeout};

use super::*;
use crate::{
    app_state::AppPaths,
    db::Database,
    documents::import::{
        BeginImportOutcome, BeginImportRequest, ImportCancellationRegistry, ImportService,
    },
    domain::{ContentAnchor, DocumentLocator, LearningAction, SelectionAnchor, TextQuote},
    learning::registry::{
        FollowupRequestContext, LearningPersistenceTarget, LearningRequestContext,
        LearningRequestRegistry, NewSelectionRequestContext,
    },
};

const TEST_TIMEOUT: Duration = Duration::from_secs(2);

#[tokio::test]
async fn concurrent_normal_operations_share_the_gate() {
    let gate = MaintenanceGate::default();
    let import = gate
        .acquire_normal(ActiveOperationKind::Import)
        .await
        .unwrap();
    let learning = gate
        .acquire_normal(ActiveOperationKind::Learning)
        .await
        .unwrap();

    assert_eq!(
        gate.status(),
        MaintenanceStatusDto {
            code: MaintenanceStatusCode::NormalOperationsActive,
            active_operations: vec![
                ActiveOperationSummaryDto {
                    kind: ActiveOperationKind::Import,
                    count: 1,
                },
                ActiveOperationSummaryDto {
                    kind: ActiveOperationKind::Learning,
                    count: 1,
                },
            ],
        }
    );
    assert!(matches!(
        gate.try_acquire_maintenance(),
        Err(GateAcquireError::Busy(_))
    ));

    drop(import);
    drop(learning);
    assert_eq!(
        gate.status().code,
        MaintenanceStatusCode::MaintenanceAvailable
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn queued_maintenance_is_writer_fair_and_blocks_late_readers() {
    let gate = MaintenanceGate::default();
    let first_reader = gate
        .acquire_normal(ActiveOperationKind::Import)
        .await
        .unwrap();

    let writer_gate = gate.clone();
    let (writer_acquired_tx, writer_acquired_rx) = oneshot::channel();
    let (release_writer_tx, release_writer_rx) = oneshot::channel();
    let writer = tokio::spawn(async move {
        let permit = writer_gate.acquire_maintenance().await.unwrap();
        writer_acquired_tx.send(()).unwrap();
        let _ = release_writer_rx.await;
        drop(permit);
    });
    wait_for_status(&gate, MaintenanceStatusCode::MaintenanceWaiting).await;

    let late_reader_gate = gate.clone();
    let (late_reader_acquired_tx, mut late_reader_acquired_rx) = oneshot::channel();
    let late_reader = tokio::spawn(async move {
        let permit = late_reader_gate
            .acquire_normal(ActiveOperationKind::Learning)
            .await
            .unwrap();
        late_reader_acquired_tx.send(()).unwrap();
        permit
    });
    yield_now().await;
    assert!(matches!(
        late_reader_acquired_rx.try_recv(),
        Err(oneshot::error::TryRecvError::Empty)
    ));
    assert!(matches!(
        gate.try_acquire_normal(ActiveOperationKind::Indexing),
        Err(GateAcquireError::Busy(status))
            if status.code == MaintenanceStatusCode::MaintenanceWaiting
    ));

    drop(first_reader);
    timeout(TEST_TIMEOUT, writer_acquired_rx)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        gate.status().code,
        MaintenanceStatusCode::MaintenanceExclusive
    );
    assert!(matches!(
        late_reader_acquired_rx.try_recv(),
        Err(oneshot::error::TryRecvError::Empty)
    ));

    release_writer_tx.send(()).unwrap();
    timeout(TEST_TIMEOUT, &mut late_reader_acquired_rx)
        .await
        .unwrap()
        .unwrap();
    drop(late_reader.await.unwrap());
    writer.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn abort_and_idempotent_release_do_not_leak_active_counts() {
    let gate = MaintenanceGate::default();
    let task_gate = gate.clone();
    let (acquired_tx, acquired_rx) = oneshot::channel();
    let task = tokio::spawn(async move {
        let _permit = task_gate
            .acquire_normal(ActiveOperationKind::Indexing)
            .await
            .unwrap();
        acquired_tx.send(()).unwrap();
        std::future::pending::<()>().await;
    });
    timeout(TEST_TIMEOUT, acquired_rx).await.unwrap().unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    wait_for_status(&gate, MaintenanceStatusCode::MaintenanceAvailable).await;

    let mut permit = gate
        .try_acquire_normal(ActiveOperationKind::Import)
        .unwrap();
    permit.release();
    permit.release();
    drop(permit);
    assert!(gate.status().active_operations.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shutdown_wakes_waiters_without_force_unlocking_existing_work() {
    let gate = MaintenanceGate::default();
    let reader = gate
        .acquire_normal(ActiveOperationKind::Learning)
        .await
        .unwrap();
    let waiting_gate = gate.clone();
    let writer = tokio::spawn(async move { waiting_gate.acquire_maintenance().await });
    wait_for_status(&gate, MaintenanceStatusCode::MaintenanceWaiting).await;

    gate.shutdown();
    assert!(matches!(
        timeout(TEST_TIMEOUT, writer).await.unwrap().unwrap(),
        Err(GateAcquireError::ShuttingDown)
    ));
    assert!(matches!(
        gate.try_acquire_normal(ActiveOperationKind::Import),
        Err(GateAcquireError::ShuttingDown)
    ));
    assert_eq!(
        gate.status().active_operations,
        vec![ActiveOperationSummaryDto {
            kind: ActiveOperationKind::Learning,
            count: 1,
        }]
    );
    drop(reader);
    assert!(gate.status().active_operations.is_empty());
    assert_eq!(
        gate.status().code,
        MaintenanceStatusCode::MaintenanceShuttingDown
    );
}

#[test]
fn active_summary_is_bounded_structural_and_redacted() {
    let gate = MaintenanceGate::default();
    let permits = (0..(MAX_SAFE_ACTIVE_OPERATION_COUNT + 7))
        .map(|_| {
            gate.try_acquire_normal(ActiveOperationKind::Import)
                .unwrap()
        })
        .collect::<Vec<_>>();

    let status = gate.status();
    assert_eq!(
        status.active_operations,
        vec![ActiveOperationSummaryDto {
            kind: ActiveOperationKind::Import,
            count: MAX_SAFE_ACTIVE_OPERATION_COUNT,
        }]
    );
    let serialized = serde_json::to_string(&status).unwrap();
    for forbidden in [
        "book title sentinel",
        "C:\\\\Users\\\\private",
        "requestOutput",
        "providerPayload",
        "profileId",
        "internalUuid",
        "imageBytes",
    ] {
        assert!(!serialized.contains(forbidden));
    }
    assert_eq!(
        serialized,
        r#"{"code":"NORMAL_OPERATIONS_ACTIVE","activeOperations":[{"kind":"import","count":99}]}"#
    );
    drop(permits);
}

#[test]
fn counter_overflow_fails_safely_without_wrapping() {
    let gate = MaintenanceGate::default();
    gate.inner.state.lock().active[ActiveOperationKind::Storage.index()] = u32::MAX;

    assert!(matches!(
        gate.try_acquire_normal(ActiveOperationKind::Storage),
        Err(GateAcquireError::CapacityExceeded)
    ));
    assert_eq!(
        gate.status().active_operations,
        vec![ActiveOperationSummaryDto {
            kind: ActiveOperationKind::Storage,
            count: MAX_SAFE_ACTIVE_OPERATION_COUNT,
        }]
    );
}

#[test]
fn import_attempt_holds_the_shared_permit_until_cancelled_terminal_cleanup() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("synthetic-app-data");
    let paths = AppPaths {
        root: root.clone(),
        books: root.join("books"),
        cache: root.join("cache"),
        logs: root.join("logs"),
        database: root.join("library.sqlite3"),
    };
    for directory in [&paths.root, &paths.books, &paths.cache, &paths.logs] {
        fs::create_dir_all(directory).unwrap();
    }
    let database = Database::open(&paths.database).unwrap();
    let gate = MaintenanceGate::default();
    let service = ImportService::with_maintenance_gate(
        database.pool().clone(),
        paths,
        ImportCancellationRegistry::default(),
        gate.clone(),
    );
    let source = temporary.path().join("synthetic.pdf");
    fs::write(&source, b"%PDF-1.7\nsynthetic maintenance test\n%%EOF").unwrap();

    let outcome = tauri::async_runtime::block_on(service.begin_import(
        BeginImportRequest::new(source.to_string_lossy().into_owned()),
        Arc::new(|_| {}),
    ))
    .unwrap();
    let BeginImportOutcome::Created { book } = outcome else {
        panic!("synthetic source must create a new import");
    };
    assert_eq!(
        gate.status().active_operations,
        vec![ActiveOperationSummaryDto {
            kind: ActiveOperationKind::Import,
            count: 1,
        }]
    );
    assert!(matches!(
        gate.try_acquire_maintenance(),
        Err(GateAcquireError::Busy(_))
    ));

    tauri::async_runtime::block_on(service.cancel_import(book.id)).unwrap();
    assert_eq!(
        gate.status().code,
        MaintenanceStatusCode::MaintenanceAvailable
    );
}

#[test]
fn import_continuation_lease_outlives_terminal_registry_removal() {
    let gate = MaintenanceGate::default();
    let registry = ImportCancellationRegistry::default();
    let book_id = uuid::Uuid::new_v4();
    let attempt_id = registry.register_with_permit(
        book_id,
        tokio_util::sync::CancellationToken::new(),
        gate.try_acquire_normal(ActiveOperationKind::Import)
            .unwrap(),
    );
    let continuation_lease = registry.operation_permit(book_id).unwrap();

    assert!(registry.remove_if_owner(book_id, attempt_id));
    assert_eq!(
        gate.status().active_operations,
        vec![ActiveOperationSummaryDto {
            kind: ActiveOperationKind::Import,
            count: 1,
        }]
    );
    assert!(matches!(
        gate.try_acquire_maintenance(),
        Err(GateAcquireError::Busy(_))
    ));

    drop(continuation_lease);
    assert_eq!(
        gate.status().code,
        MaintenanceStatusCode::MaintenanceAvailable
    );
}

#[test]
fn learning_registry_holds_permits_for_new_selection_and_followup_until_terminal() {
    let gate = MaintenanceGate::default();
    let registry = LearningRequestRegistry::with_maintenance_gate(gate.clone());
    let first = registry.create(synthetic_learning_context()).unwrap();
    let second = registry.create(synthetic_followup_context()).unwrap();
    assert_eq!(
        gate.status().active_operations,
        vec![ActiveOperationSummaryDto {
            kind: ActiveOperationKind::Learning,
            count: 2,
        }]
    );

    registry.cancel(first.request_id).unwrap();
    assert_eq!(gate.status().active_operations[0].count, 1);
    registry
        .fail(
            second.request_id,
            &crate::errors::AppError::new(crate::errors::AppErrorCode::ProviderUnavailable),
        )
        .unwrap();
    assert_eq!(
        gate.status().code,
        MaintenanceStatusCode::MaintenanceAvailable
    );
}

#[test]
fn learning_worker_lease_outlives_cancelled_terminal_registry_state() {
    let gate = MaintenanceGate::default();
    let registry = LearningRequestRegistry::with_maintenance_gate(gate.clone());
    let request = registry.create(synthetic_learning_context()).unwrap();
    let worker_lease = registry.operation_permit(request.request_id).unwrap();

    registry.cancel(request.request_id).unwrap();
    assert_eq!(
        gate.status().active_operations,
        vec![ActiveOperationSummaryDto {
            kind: ActiveOperationKind::Learning,
            count: 1,
        }]
    );
    assert!(matches!(
        gate.try_acquire_maintenance(),
        Err(GateAcquireError::Busy(_))
    ));

    drop(worker_lease);
    assert_eq!(
        gate.status().code,
        MaintenanceStatusCode::MaintenanceAvailable
    );
}

async fn wait_for_status(gate: &MaintenanceGate, expected: MaintenanceStatusCode) {
    timeout(TEST_TIMEOUT, async {
        loop {
            if gate.status().code == expected {
                return;
            }
            yield_now().await;
        }
    })
    .await
    .unwrap();
}

fn synthetic_learning_context() -> Arc<LearningRequestContext> {
    let section_id = uuid::Uuid::new_v4();
    Arc::new(LearningRequestContext {
        target: LearningPersistenceTarget::NewSelection(NewSelectionRequestContext {
            book_id: uuid::Uuid::new_v4(),
            section_id,
            anchor: ContentAnchor::Text {
                selection: SelectionAnchor {
                    locator: DocumentLocator::pdf(1, 1, None).unwrap(),
                    quote: TextQuote::new(
                        "synthetic selection".to_owned(),
                        String::new(),
                        String::new(),
                    )
                    .unwrap(),
                    section_id: Some(section_id),
                },
            },
            selected_text: Some("synthetic selection".to_owned()),
            action: LearningAction::Explain,
            question: "synthetic question".to_owned(),
        }),
        provider_profile_id: uuid::Uuid::new_v4(),
        model_id: "synthetic-model".to_owned(),
        available_citations: Vec::new(),
    })
}

fn synthetic_followup_context() -> Arc<LearningRequestContext> {
    Arc::new(LearningRequestContext {
        target: LearningPersistenceTarget::SelectionFollowup(FollowupRequestContext {
            book_id: uuid::Uuid::new_v4(),
            conversation_id: uuid::Uuid::new_v4(),
            expected_next_ordinal: 2,
            question: "synthetic followup".to_owned(),
        }),
        provider_profile_id: uuid::Uuid::new_v4(),
        model_id: "synthetic-model".to_owned(),
        available_citations: Vec::new(),
    })
}
