use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use sqlx::Row;
use tokio::sync::Barrier;
use uuid::Uuid;

use crate::{
    db::{Database, overview::OverviewReadObserver},
    domain::{ActiveOperationKind, LearningOverviewErrorCode, MaintenanceStatusCode},
    maintenance::gate::MaintenanceGate,
};

use super::{LearningOverviewService, LearningOverviewServiceError};

const TIMESTAMP: &str = "2026-08-06T00:00:00.000Z";

#[test]
fn overview_snapshot_holds_a_normal_lease_and_releases_it_on_success_error_and_cancel() {
    let temporary = tempfile::tempdir().unwrap();
    let database = Database::open(temporary.path().join("service-gate.sqlite3")).unwrap();
    tauri::async_runtime::block_on(async {
        let book_id = insert_empty_ready_book(database.pool()).await;
        let gate = MaintenanceGate::default();
        let service = LearningOverviewService::new(database.pool().clone(), gate.clone());
        let observer = Arc::new(PauseObserver::new());
        let task_service = service.clone();
        let task_observer = observer.clone();
        let read = tokio::spawn(async move {
            task_service
                .get_observed(book_id, task_observer.as_ref())
                .await
        });
        observer.reached.wait().await;
        let status = gate.status();
        assert_eq!(status.code, MaintenanceStatusCode::NormalOperationsActive);
        assert_eq!(status.active_operations.len(), 1);
        assert_eq!(
            status.active_operations[0].kind,
            ActiveOperationKind::Learning
        );
        assert_eq!(status.active_operations[0].count, 1);

        let maintenance_gate = gate.clone();
        let maintenance = tokio::spawn(async move { maintenance_gate.acquire_maintenance().await });
        tokio::time::timeout(Duration::from_secs(5), async {
            while gate.status().code != MaintenanceStatusCode::MaintenanceWaiting {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("maintenance waiter registered behind the snapshot");
        assert_eq!(
            gate.status().code,
            MaintenanceStatusCode::MaintenanceWaiting
        );
        observer.resume.wait().await;
        read.await.unwrap().unwrap();
        let permit = tokio::time::timeout(Duration::from_secs(5), maintenance)
            .await
            .expect("maintenance acquired after the snapshot")
            .unwrap()
            .unwrap();
        assert_eq!(
            gate.status().code,
            MaintenanceStatusCode::MaintenanceExclusive
        );
        drop(permit);

        let missing = service.get(Uuid::new_v4()).await.unwrap_err();
        assert_eq!(missing.stable_code(), LearningOverviewErrorCode::NotFound);
        drop(gate.try_acquire_maintenance().unwrap());

        let cancel_observer = Arc::new(PauseObserver::new());
        let task_service = service.clone();
        let task_observer = cancel_observer.clone();
        let cancelled = tokio::spawn(async move {
            task_service
                .get_observed(book_id, task_observer.as_ref())
                .await
        });
        cancel_observer.reached.wait().await;
        cancelled.abort();
        assert!(cancelled.await.unwrap_err().is_cancelled());
        drop(gate.try_acquire_maintenance().unwrap());
    });
}

#[test]
fn overview_service_has_zero_provider_credential_network_and_database_write_effects() {
    let temporary = tempfile::tempdir().unwrap();
    let database = Database::open(temporary.path().join("service-effects.sqlite3")).unwrap();
    tauri::async_runtime::block_on(async {
        let book_id = insert_empty_ready_book(database.pool()).await;
        let service =
            LearningOverviewService::new(database.pool().clone(), MaintenanceGate::default());
        let forbidden = ForbiddenSideEffectCounters::default();
        let mut witness = database.pool().acquire().await.unwrap();
        let before_version: i64 = sqlx::query_scalar("PRAGMA data_version")
            .fetch_one(&mut *witness)
            .await
            .unwrap();
        let before_rows = owned_row_count(database.pool()).await;

        let overview = service.get(book_id).await.unwrap();
        assert_eq!(overview.book_id, book_id);

        let after_version: i64 = sqlx::query_scalar("PRAGMA data_version")
            .fetch_one(&mut *witness)
            .await
            .unwrap();
        assert_eq!(after_version, before_version);
        assert_eq!(owned_row_count(database.pool()).await, before_rows);
        assert_eq!(forbidden.provider_runtime.load(Ordering::SeqCst), 0);
        assert_eq!(forbidden.credential_reads.load(Ordering::SeqCst), 0);
        assert_eq!(forbidden.network_requests.load(Ordering::SeqCst), 0);
        assert_eq!(forbidden.database_writes.load(Ordering::SeqCst), 0);

        let service_source = include_str!("overview.rs");
        let database_source = include_str!("../db/overview.rs");
        for forbidden_dependency in [
            "ProviderRuntime",
            "CredentialStore",
            "credential_store",
            "reqwest",
            "Utc::now",
            "HashMap",
            "rand::",
        ] {
            assert!(!service_source.contains(forbidden_dependency));
            assert!(!database_source.contains(forbidden_dependency));
        }
        for forbidden_write in ["INSERT INTO", "UPDATE ", "DELETE FROM", "REPLACE INTO"] {
            assert!(!database_source.contains(forbidden_write));
        }
        assert_eq!(
            format!("{service:?}"),
            "LearningOverviewService(local-database-only)"
        );
    });
}

#[test]
fn exclusive_maintenance_maps_to_one_stable_busy_code() {
    let temporary = tempfile::tempdir().unwrap();
    let database = Database::open(temporary.path().join("service-busy.sqlite3")).unwrap();
    tauri::async_runtime::block_on(async {
        let book_id = insert_empty_ready_book(database.pool()).await;
        let gate = MaintenanceGate::default();
        let _maintenance = gate.try_acquire_maintenance().unwrap();
        let service = LearningOverviewService::new(database.pool().clone(), gate);
        let error = service.get(book_id).await.unwrap_err();
        assert!(matches!(&error, LearningOverviewServiceError::Busy));
        assert_eq!(error.stable_code(), LearningOverviewErrorCode::Busy);
    });
}

struct PauseObserver {
    reached: Arc<Barrier>,
    resume: Arc<Barrier>,
}

impl PauseObserver {
    fn new() -> Self {
        Self {
            reached: Arc::new(Barrier::new(2)),
            resume: Arc::new(Barrier::new(2)),
        }
    }
}

#[async_trait]
impl OverviewReadObserver for PauseObserver {
    async fn after_query(&self, query_count: u8) {
        if query_count == 1 {
            self.reached.wait().await;
            self.resume.wait().await;
        }
    }
}

#[derive(Default)]
struct ForbiddenSideEffectCounters {
    provider_runtime: AtomicUsize,
    credential_reads: AtomicUsize,
    network_requests: AtomicUsize,
    database_writes: AtomicUsize,
}

async fn insert_empty_ready_book(pool: &sqlx::SqlitePool) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO books (id, sha256, title, format, original_filename, stored_path, import_status, created_at, updated_at) VALUES (?, ?, 'Service book', 'pdf', 'service.pdf', ?, 'ready', ?, ?)",
    )
    .bind(id.to_string())
    .bind("a".repeat(64))
    .bind(format!("books/{id}/original.pdf"))
    .bind(TIMESTAMP)
    .bind(TIMESTAMP)
    .execute(pool)
    .await
    .unwrap();
    id
}

async fn owned_row_count(pool: &sqlx::SqlitePool) -> i64 {
    let row = sqlx::query(
        "SELECT (SELECT COUNT(*) FROM conversations) AS conversations, (SELECT COUNT(*) FROM messages) AS messages, (SELECT COUNT(*) FROM annotations) AS annotations, (SELECT COUNT(*) FROM index_corrections) AS corrections, (SELECT COUNT(*) FROM index_search_chunks) AS index_chunks",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    [
        row.try_get::<i64, _>("conversations").unwrap(),
        row.try_get::<i64, _>("messages").unwrap(),
        row.try_get::<i64, _>("annotations").unwrap(),
        row.try_get::<i64, _>("corrections").unwrap(),
        row.try_get::<i64, _>("index_chunks").unwrap(),
    ]
    .into_iter()
    .sum()
}
