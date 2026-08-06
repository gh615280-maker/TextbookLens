use std::{
    collections::HashMap,
    fs,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

use async_trait::async_trait;
use secrecy::{ExposeSecret, SecretString};
use sqlx::{Row, SqlitePool};
use textbooklens_lib::{
    app_state::AppPaths,
    credentials::CredentialStore,
    db::Database,
    domain::{ActiveOperationKind, ProviderKind, RemoteCleanupHandle},
    errors::{AppError, AppErrorCode, AppErrorDto, AppResult},
    indexing::remote_cleanup::{
        RemoteResourceCleaner, store_remote_handle, sweep_remote_resource_ids,
    },
    maintenance::{
        delete_book::{
            DeleteBookError, DeleteBookService, DeleteBoundary, DeleteFaultInjector,
            DeleteInjectedCrash, recover_pending_deletions_async,
        },
        gate::MaintenanceGate,
        journal::{DeleteJournal, JournalStore, NoJournalFault, RelativePathToken},
        storage::{APP_DATA_DIRECTORY_NAME, prepare_app_data_paths},
    },
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const TIMESTAMP: &str = "2026-08-06T00:00:00.000Z";

#[tokio::test]
async fn complete_delete_removes_every_local_relation_and_owned_file_but_not_decoys_or_originals() {
    let fixture = FullFixture::new().await;
    let decoy_before = book_scoped_counts(fixture.database.pool(), fixture.decoy.book_id).await;
    let service = fixture.service();

    let outcome = service.delete_book(fixture.target.book_id).await.unwrap();
    assert_eq!(outcome.detached_remote_resource_count(), 1);
    let cleaner = FakeCleaner::default();
    let summary = sweep_remote_resource_ids(
        fixture.database.pool(),
        fixture.store.as_ref(),
        &cleaner,
        outcome.detached_remote_resource_ids(),
        CancellationToken::new(),
    )
    .await
    .unwrap();
    assert_eq!(summary.succeeded, 1);
    assert_eq!(cleaner.calls.load(Ordering::SeqCst), 1);

    assert!(!book_exists(fixture.database.pool(), fixture.target.book_id).await);
    assert!(
        book_scoped_counts(fixture.database.pool(), fixture.target.book_id)
            .await
            .values()
            .all(|count| *count == 0)
    );
    assert_eq!(
        fts_match_count(fixture.database.pool(), &fixture.target.local_sentinel).await,
        0
    );
    assert_eq!(
        fts_index_match_count(fixture.database.pool(), &fixture.target.index_sentinel).await,
        0
    );
    assert_eq!(
        book_scoped_counts(fixture.database.pool(), fixture.decoy.book_id).await,
        decoy_before
    );
    assert_eq!(
        fts_match_count(fixture.database.pool(), &fixture.decoy.local_sentinel).await,
        1
    );
    assert_eq!(
        fts_index_match_count(fixture.database.pool(), &fixture.decoy.index_sentinel).await,
        1
    );

    assert!(!fixture.target.book_directory.exists());
    assert!(!fixture.target.scratch_directory.exists());
    assert_eq!(
        fs::read(&fixture.decoy.owned_source).unwrap(),
        fixture.decoy.owned_bytes
    );
    assert_eq!(
        fs::read(&fixture.decoy.scratch_file).unwrap(),
        b"decoy-render"
    );
    assert_eq!(
        fs::read(&fixture.target.user_source).unwrap(),
        fixture.target.user_bytes
    );
    assert_eq!(
        fs::read(&fixture.decoy.user_source).unwrap(),
        fixture.decoy.user_bytes
    );

    let remote = sqlx::query(
        "SELECT book_id, run_id, page_id, cleanup_status FROM provider_remote_resources WHERE id = ?",
    )
    .bind(fixture.remote_resource_id.to_string())
    .fetch_one(fixture.database.pool())
    .await
    .unwrap();
    assert!(remote.get::<Option<String>, _>("book_id").is_none());
    assert!(remote.get::<Option<String>, _>("run_id").is_none());
    assert!(remote.get::<Option<String>, _>("page_id").is_none());
    assert_eq!(remote.get::<String, _>("cleanup_status"), "succeeded");
    assert_eq!(fixture.store.entry_count(), 1);
    let decoy_remote = sqlx::query(
        "SELECT book_id, run_id, page_id, cleanup_status FROM provider_remote_resources WHERE id = ?",
    )
    .bind(fixture.decoy_remote_resource_id.to_string())
    .fetch_one(fixture.database.pool())
    .await
    .unwrap();
    assert_eq!(
        decoy_remote.get::<String, _>("book_id"),
        fixture.decoy.book_id.to_string()
    );
    assert_eq!(
        decoy_remote.get::<String, _>("run_id"),
        fixture.decoy.run_id.to_string()
    );
    assert_eq!(
        decoy_remote.get::<String, _>("page_id"),
        fixture.decoy.page_id.to_string()
    );
    assert_eq!(decoy_remote.get::<String, _>("cleanup_status"), "pending");
    assert_no_pending_delete_artifacts(&fixture.paths);
    assert!(
        sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(fixture.database.pool())
            .await
            .unwrap()
            .is_empty()
    );

    assert_eq!(
        service
            .delete_book(fixture.target.book_id)
            .await
            .unwrap_err()
            .code,
        AppErrorCode::NotFound
    );
    recover_pending_deletions_async(fixture.database.pool(), &fixture.paths)
        .await
        .unwrap();
    recover_pending_deletions_async(fixture.database.pool(), &fixture.paths)
        .await
        .unwrap();
    assert_no_pending_delete_artifacts(&fixture.paths);
}

#[tokio::test]
async fn every_mutation_fault_recovers_to_exactly_one_pre_or_post_commit_state() {
    let harness = CrashHarness::new().await;
    let boundaries = DeleteBoundary::fault_matrix(2);
    assert_eq!(boundaries.len(), 45);

    for (ordinal, boundary) in boundaries.into_iter().enumerate() {
        let seed = harness.seed_minimal(ordinal).await;
        let service = harness.service();
        let error = service
            .delete_book_with_fault(seed.book_id, &FailAt::new(boundary))
            .await
            .unwrap_err();
        assert_eq!(error.injected_boundary(), Some(boundary));

        recover_pending_deletions_async(harness.database.pool(), &harness.paths)
            .await
            .unwrap();
        recover_pending_deletions_async(harness.database.pool(), &harness.paths)
            .await
            .unwrap();
        let exists = book_exists(harness.database.pool(), seed.book_id).await;
        if boundary.is_after_database_commit() {
            assert!(!exists, "post-commit fault left a book at {boundary:?}");
            assert!(!seed.book_directory.exists());
            assert!(!seed.scratch_directory.exists());
        } else {
            assert!(exists, "pre-commit fault lost a book at {boundary:?}");
            assert_eq!(fs::read(&seed.owned_source).unwrap(), seed.owned_bytes);
            assert_eq!(fs::read(&seed.scratch_file).unwrap(), b"crash-render");
            service.delete_book(seed.book_id).await.unwrap();
        }
        assert_no_pending_delete_artifacts(&harness.paths);
        assert_eq!(
            fs::read(&seed.user_source).unwrap(),
            seed.user_bytes,
            "original source changed at {boundary:?}"
        );
    }
}

#[tokio::test]
async fn malicious_stored_paths_fail_before_move_or_database_mutation_and_are_redacted() {
    let fixture = FullFixture::new().await;
    let service = fixture.service();
    let outside = fixture
        .temporary
        .path()
        .join("outside-private-sentinel.pdf");
    fs::write(&outside, b"outside-private-bytes").unwrap();
    let expected = format!("books/{}/original.pdf", fixture.target.book_id);
    let malicious = [
        outside.to_string_lossy().into_owned(),
        format!("books/{}/../outside.pdf", fixture.target.book_id),
        format!(r"books\{}\original.pdf", fixture.target.book_id),
        r"\\server\share\private.pdf".to_owned(),
        r"\\?\C:\private.pdf".to_owned(),
        expected.to_ascii_uppercase(),
    ];
    for stored_path in malicious {
        sqlx::query("UPDATE books SET stored_path = ? WHERE id = ?")
            .bind(&stored_path)
            .bind(fixture.target.book_id.to_string())
            .execute(fixture.database.pool())
            .await
            .unwrap();
        let error = service
            .delete_book(fixture.target.book_id)
            .await
            .unwrap_err();
        assert_eq!(error.code, AppErrorCode::InvalidInput);
        let dto = serde_json::to_string(&AppErrorDto::from(error)).unwrap();
        assert!(!dto.contains("outside-private"));
        assert!(!dto.contains(&stored_path));
        assert!(book_exists(fixture.database.pool(), fixture.target.book_id).await);
        assert!(fixture.target.book_directory.exists());
        assert_eq!(fs::read(&outside).unwrap(), b"outside-private-bytes");
        assert_no_pending_delete_artifacts(&fixture.paths);
    }
    sqlx::query("UPDATE books SET stored_path = ? WHERE id = ?")
        .bind(expected)
        .bind(fixture.target.book_id.to_string())
        .execute(fixture.database.pool())
        .await
        .unwrap();
    service.delete_book(fixture.target.book_id).await.unwrap();
    assert_eq!(fs::read(outside).unwrap(), b"outside-private-bytes");
}

#[tokio::test]
async fn malicious_journal_cannot_delete_a_decoy_page_or_partially_apply_earlier_entries() {
    let fixture = FullFixture::new().await;
    let decoy_before = book_scoped_counts(fixture.database.pool(), fixture.decoy.book_id).await;
    let orphan_page_id = Uuid::nil();
    let orphan_directory = fixture
        .paths
        .indexing_scratch()
        .join(orphan_page_id.to_string());
    let orphan_attempt_id = Uuid::new_v4();
    let orphan_file = orphan_directory.join(format!("{orphan_attempt_id}.render"));
    fs::create_dir(&orphan_directory).unwrap();
    fs::write(&orphan_file, b"orphan-must-not-be-partially-deleted").unwrap();

    let first_journal_id = Uuid::from_u128(1);
    let malicious_journal_id = Uuid::from_u128(2);
    let first_journal = DeleteJournal::from_sources(
        first_journal_id,
        Uuid::from_u128(3),
        vec![RelativePathToken::new(format!("cache/indexing-pages/{orphan_page_id}")).unwrap()],
    )
    .unwrap();
    let malicious_journal = DeleteJournal::from_sources(
        malicious_journal_id,
        Uuid::from_u128(4),
        vec![
            RelativePathToken::new(format!("cache/indexing-pages/{}", fixture.decoy.page_id))
                .unwrap(),
        ],
    )
    .unwrap();
    let store = JournalStore::open(&fixture.paths).unwrap();
    store.write_intent(&first_journal, &NoJournalFault).unwrap();
    store.create_journal_trash_root(first_journal_id).unwrap();
    store
        .write_intent(&malicious_journal, &NoJournalFault)
        .unwrap();
    store
        .create_journal_trash_root(malicious_journal_id)
        .unwrap();

    let error = recover_pending_deletions_async(fixture.database.pool(), &fixture.paths)
        .await
        .unwrap_err();
    assert_eq!(error.code, AppErrorCode::InvalidInput);
    assert_eq!(
        fs::read(orphan_file).unwrap(),
        b"orphan-must-not-be-partially-deleted"
    );
    assert_eq!(
        fs::read(&fixture.decoy.scratch_file).unwrap(),
        b"decoy-render"
    );
    assert_eq!(
        book_scoped_counts(fixture.database.pool(), fixture.decoy.book_id).await,
        decoy_before
    );
    assert_eq!(
        store.list_intents().unwrap(),
        vec![first_journal, malicious_journal]
    );
}

#[tokio::test]
async fn active_operations_and_nonterminal_rows_require_explicit_terminal_acknowledgment() {
    let fixture = FullFixture::new().await;
    let service = fixture.service();

    let import = fixture
        .gate
        .try_acquire_normal(ActiveOperationKind::Import)
        .unwrap();
    assert_eq!(
        service
            .delete_book(fixture.target.book_id)
            .await
            .unwrap_err()
            .code,
        AppErrorCode::RequestConflict
    );
    drop(import);

    let learning = fixture
        .gate
        .try_acquire_normal(ActiveOperationKind::Learning)
        .unwrap();
    assert_eq!(
        service
            .delete_book(fixture.target.book_id)
            .await
            .unwrap_err()
            .code,
        AppErrorCode::RequestConflict
    );
    drop(learning);

    sqlx::query("UPDATE index_runs SET status = 'running', completed_at = NULL WHERE id = ?")
        .bind(fixture.target.run_id.to_string())
        .execute(fixture.database.pool())
        .await
        .unwrap();
    assert_eq!(
        service
            .delete_book(fixture.target.book_id)
            .await
            .unwrap_err()
            .code,
        AppErrorCode::RequestConflict
    );
    sqlx::query(
        "UPDATE index_runs SET status = 'cancelled', cancel_requested_at = ?, completed_at = ? WHERE id = ?",
    )
    .bind(TIMESTAMP)
    .bind(TIMESTAMP)
    .bind(fixture.target.run_id.to_string())
    .execute(fixture.database.pool())
    .await
    .unwrap();

    sqlx::query("UPDATE books SET import_status = 'queued' WHERE id = ?")
        .bind(fixture.target.book_id.to_string())
        .execute(fixture.database.pool())
        .await
        .unwrap();
    assert_eq!(
        service
            .delete_book(fixture.target.book_id)
            .await
            .unwrap_err()
            .code,
        AppErrorCode::RequestConflict
    );
    sqlx::query("UPDATE books SET import_status = 'failed' WHERE id = ?")
        .bind(fixture.target.book_id.to_string())
        .execute(fixture.database.pool())
        .await
        .unwrap();

    service.delete_book(fixture.target.book_id).await.unwrap();
    assert!(!book_exists(fixture.database.pool(), fixture.target.book_id).await);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_same_book_delete_has_one_winner_and_a_stable_loser() {
    let fixture = FullFixture::new().await;
    let service = fixture.service();
    let (entered_tx, entered_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let blocker = Arc::new(BlockingFault {
        entered: Mutex::new(Some(entered_tx)),
        release: Mutex::new(release_rx),
    });
    let first_service = service.clone();
    let first_fault = blocker.clone();
    let book_id = fixture.target.book_id;
    let first = tokio::spawn(async move {
        first_service
            .delete_book_with_fault(book_id, first_fault.as_ref())
            .await
    });
    tokio::task::spawn_blocking(move || entered_rx.recv().unwrap())
        .await
        .unwrap();

    let loser = service.delete_book(book_id).await.unwrap_err();
    assert_eq!(loser.code, AppErrorCode::RequestConflict);
    release_tx.send(()).unwrap();
    first.await.unwrap().unwrap();
    assert!(!book_exists(fixture.database.pool(), book_id).await);
}

#[tokio::test]
async fn detached_remote_failure_survives_restart_and_success_is_never_deleted_twice() {
    let fixture = FullFixture::new().await;
    let service = fixture.service();
    let outcome = service.delete_book(fixture.target.book_id).await.unwrap();
    let ids = outcome.detached_remote_resource_ids().to_vec();
    let cleaner = Arc::new(FakeCleaner::default());
    cleaner.failures_remaining.store(1, Ordering::SeqCst);
    let first = sweep_remote_resource_ids(
        fixture.database.pool(),
        fixture.store.as_ref(),
        cleaner.as_ref(),
        &ids,
        CancellationToken::new(),
    )
    .await
    .unwrap();
    assert_eq!(first.failed, 1);
    assert_eq!(cleaner.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        cleanup_status(fixture.database.pool(), fixture.remote_resource_id).await,
        "failed"
    );

    let database_path = fixture.paths.database.clone();
    drop(service);
    fixture.database.pool().close().await;
    let reopened = tokio::task::spawn_blocking(move || Database::open(database_path))
        .await
        .unwrap()
        .unwrap();
    recover_pending_deletions_async(reopened.pool(), &fixture.paths)
        .await
        .unwrap();

    let (second, competing) = tokio::join!(
        sweep_remote_resource_ids(
            reopened.pool(),
            fixture.store.as_ref(),
            cleaner.as_ref(),
            &ids,
            CancellationToken::new(),
        ),
        sweep_remote_resource_ids(
            reopened.pool(),
            fixture.store.as_ref(),
            cleaner.as_ref(),
            &ids,
            CancellationToken::new(),
        )
    );
    let claimed = second.unwrap().succeeded + competing.unwrap().succeeded;
    assert_eq!(claimed, 1);
    assert_eq!(cleaner.calls.load(Ordering::SeqCst), 2);
    assert_eq!(
        cleanup_status(reopened.pool(), fixture.remote_resource_id).await,
        "succeeded"
    );
    assert_eq!(
        cleanup_status(reopened.pool(), fixture.decoy_remote_resource_id).await,
        "pending"
    );
    sweep_remote_resource_ids(
        reopened.pool(),
        fixture.store.as_ref(),
        cleaner.as_ref(),
        &ids,
        CancellationToken::new(),
    )
    .await
    .unwrap();
    assert_eq!(cleaner.calls.load(Ordering::SeqCst), 2);
}

#[cfg(windows)]
#[tokio::test]
async fn windows_junction_and_race_replacement_are_rejected_without_touching_outside_data() {
    use std::process::Command;

    let fixture = FullFixture::new().await;
    let service = fixture.service();
    let outside = fixture.temporary.path().join("outside-junction-target");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("private-sentinel"), b"outside-untouched").unwrap();
    fs::remove_dir_all(&fixture.target.book_directory).unwrap();
    create_junction(&outside, &fixture.target.book_directory);

    let error = service
        .delete_book(fixture.target.book_id)
        .await
        .unwrap_err();
    assert_eq!(error.code, AppErrorCode::InvalidInput);
    assert!(book_exists(fixture.database.pool(), fixture.target.book_id).await);
    assert_eq!(
        fs::read(outside.join("private-sentinel")).unwrap(),
        b"outside-untouched"
    );
    fs::remove_dir(&fixture.target.book_directory).unwrap();

    let replacement = fixture.temporary.path().join("outside-race-target");
    fs::create_dir(&replacement).unwrap();
    fs::write(replacement.join("race-sentinel"), b"race-untouched").unwrap();
    fs::create_dir_all(fixture.target.book_directory.join("derived")).unwrap();
    fs::write(&fixture.target.owned_source, fixture.target.owned_bytes).unwrap();
    let injector = ReplaceWithJunction {
        boundary: DeleteBoundary::BeforeStageMove(0),
        source: fixture.target.book_directory.clone(),
        outside: replacement.clone(),
        replaced: AtomicBool::new(false),
    };
    let raced = service
        .delete_book_with_fault(fixture.target.book_id, &injector)
        .await
        .unwrap_err();
    assert!(matches!(raced, DeleteBookError::App(_)));
    assert!(book_exists(fixture.database.pool(), fixture.target.book_id).await);
    assert_eq!(
        fs::read(replacement.join("race-sentinel")).unwrap(),
        b"race-untouched"
    );
    if fixture.target.book_directory.exists() {
        fs::remove_dir(&fixture.target.book_directory).unwrap();
    }
    fs::create_dir_all(fixture.target.book_directory.join("derived")).unwrap();
    fs::write(&fixture.target.owned_source, fixture.target.owned_bytes).unwrap();
    recover_pending_deletions_async(fixture.database.pool(), &fixture.paths)
        .await
        .unwrap();
    assert_no_pending_delete_artifacts(&fixture.paths);

    fn create_junction(source: &std::path::Path, target: &std::path::Path) {
        let output = Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(target)
            .arg(source)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

struct FailAt {
    boundary: DeleteBoundary,
    fired: AtomicBool,
}

impl FailAt {
    fn new(boundary: DeleteBoundary) -> Self {
        Self {
            boundary,
            fired: AtomicBool::new(false),
        }
    }
}

impl DeleteFaultInjector for FailAt {
    fn checkpoint(&self, boundary: DeleteBoundary) -> Result<(), DeleteInjectedCrash> {
        if boundary == self.boundary && !self.fired.swap(true, Ordering::SeqCst) {
            Err(DeleteInjectedCrash)
        } else {
            Ok(())
        }
    }
}

struct BlockingFault {
    entered: Mutex<Option<std::sync::mpsc::Sender<()>>>,
    release: Mutex<std::sync::mpsc::Receiver<()>>,
}

impl DeleteFaultInjector for BlockingFault {
    fn checkpoint(&self, boundary: DeleteBoundary) -> Result<(), DeleteInjectedCrash> {
        if boundary == DeleteBoundary::TrashRootCreated
            && let Some(entered) = self.entered.lock().unwrap().take()
        {
            entered.send(()).unwrap();
            self.release.lock().unwrap().recv().unwrap();
        }
        Ok(())
    }
}

#[cfg(windows)]
struct ReplaceWithJunction {
    boundary: DeleteBoundary,
    source: std::path::PathBuf,
    outside: std::path::PathBuf,
    replaced: AtomicBool,
}

#[cfg(windows)]
impl DeleteFaultInjector for ReplaceWithJunction {
    fn checkpoint(&self, boundary: DeleteBoundary) -> Result<(), DeleteInjectedCrash> {
        if boundary == self.boundary && !self.replaced.swap(true, Ordering::SeqCst) {
            fs::remove_dir_all(&self.source).unwrap();
            let output = std::process::Command::new("cmd")
                .args(["/C", "mklink", "/J"])
                .arg(&self.source)
                .arg(&self.outside)
                .output()
                .unwrap();
            assert!(output.status.success());
        }
        Ok(())
    }
}

#[derive(Default)]
struct TestStore {
    values: Mutex<HashMap<String, SecretString>>,
}

impl TestStore {
    fn entry_count(&self) -> usize {
        self.values.lock().unwrap().len()
    }
}

#[async_trait]
impl CredentialStore for TestStore {
    async fn set(&self, key: &str, value: SecretString) -> AppResult<()> {
        self.values.lock().unwrap().insert(key.to_owned(), value);
        Ok(())
    }

    async fn get(&self, key: &str) -> AppResult<SecretString> {
        self.values
            .lock()
            .unwrap()
            .get(key)
            .cloned()
            .ok_or_else(|| AppError::new(AppErrorCode::CredentialStoreError))
    }

    async fn delete(&self, key: &str) -> AppResult<()> {
        self.values.lock().unwrap().remove(key);
        Ok(())
    }
}

#[derive(Default)]
struct FakeCleaner {
    calls: AtomicUsize,
    failures_remaining: AtomicUsize,
}

#[async_trait]
impl RemoteResourceCleaner for FakeCleaner {
    async fn delete(
        &self,
        _pool: &SqlitePool,
        _profile_id: Uuid,
        handle: &RemoteCleanupHandle,
        _cancel: CancellationToken,
    ) -> AppResult<()> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        assert_eq!(handle.provider(), &ProviderKind::OpenAi);
        assert!(
            handle
                .opaque_id()
                .expose_secret()
                .starts_with("opaque-synthetic-")
        );
        if self.failures_remaining.load(Ordering::SeqCst) > 0 {
            self.failures_remaining.fetch_sub(1, Ordering::SeqCst);
            return Err(AppError::new(AppErrorCode::ProviderUnavailable));
        }
        Ok(())
    }
}

struct FullFixture {
    temporary: tempfile::TempDir,
    paths: AppPaths,
    database: Database,
    gate: MaintenanceGate,
    store: Arc<TestStore>,
    target: BookSeed,
    decoy: BookSeed,
    remote_resource_id: Uuid,
    decoy_remote_resource_id: Uuid,
}

impl FullFixture {
    async fn new() -> Self {
        let (temporary, paths, database) = synthetic_database().await;
        let profile_id = seed_profile(database.pool()).await;
        let target = seed_full_book(
            database.pool(),
            &paths,
            &temporary,
            profile_id,
            "target",
            b"target-owned-bytes",
            b"target-user-original",
        )
        .await;
        let decoy = seed_full_book(
            database.pool(),
            &paths,
            &temporary,
            profile_id,
            "decoy",
            b"decoy-owned-bytes",
            b"decoy-user-original",
        )
        .await;
        let store = Arc::new(TestStore::default());
        let remote_resource_id = store_remote_handle(
            database.pool(),
            store.as_ref(),
            target.page_id,
            &RemoteCleanupHandle::new(
                ProviderKind::OpenAi,
                SecretString::from(format!("opaque-synthetic-{}", target.book_id)),
            ),
        )
        .await
        .unwrap();
        let decoy_remote_resource_id = store_remote_handle(
            database.pool(),
            store.as_ref(),
            decoy.page_id,
            &RemoteCleanupHandle::new(
                ProviderKind::OpenAi,
                SecretString::from(format!("opaque-synthetic-{}", decoy.book_id)),
            ),
        )
        .await
        .unwrap();
        Self {
            temporary,
            paths,
            database,
            gate: MaintenanceGate::default(),
            store,
            target,
            decoy,
            remote_resource_id,
            decoy_remote_resource_id,
        }
    }

    fn service(&self) -> DeleteBookService {
        DeleteBookService::new(
            self.database.pool().clone(),
            self.paths.clone(),
            self.gate.clone(),
        )
    }
}

struct CrashHarness {
    _temporary: tempfile::TempDir,
    paths: AppPaths,
    database: Database,
    gate: MaintenanceGate,
}

impl CrashHarness {
    async fn new() -> Self {
        let (temporary, paths, database) = synthetic_database().await;
        Self {
            _temporary: temporary,
            paths,
            database,
            gate: MaintenanceGate::default(),
        }
    }

    fn service(&self) -> DeleteBookService {
        DeleteBookService::new(
            self.database.pool().clone(),
            self.paths.clone(),
            self.gate.clone(),
        )
    }

    async fn seed_minimal(&self, ordinal: usize) -> BookSeed {
        seed_minimal_book(self.database.pool(), &self.paths, &self._temporary, ordinal).await
    }
}

struct BookSeed {
    book_id: Uuid,
    run_id: Uuid,
    page_id: Uuid,
    user_source: std::path::PathBuf,
    user_bytes: &'static [u8],
    book_directory: std::path::PathBuf,
    owned_source: std::path::PathBuf,
    owned_bytes: &'static [u8],
    scratch_directory: std::path::PathBuf,
    scratch_file: std::path::PathBuf,
    local_sentinel: String,
    index_sentinel: String,
}

async fn synthetic_database() -> (tempfile::TempDir, AppPaths, Database) {
    tokio::task::spawn_blocking(|| {
        let temporary = tempfile::tempdir().unwrap();
        let prepared =
            prepare_app_data_paths(&temporary.path().join(APP_DATA_DIRECTORY_NAME)).unwrap();
        let paths = AppPaths {
            root: prepared.root,
            books: prepared.books,
            cache: prepared.cache,
            logs: prepared.logs,
            database: prepared.database,
        };
        let database = Database::open(&paths.database).unwrap();
        (temporary, paths, database)
    })
    .await
    .unwrap()
}

async fn seed_profile(pool: &SqlitePool) -> Uuid {
    let profile_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO provider_profiles (id, provider_kind, display_name, model_id, context_window_tokens, created_at, updated_at, validated_at) VALUES (?, 'openai', 'Synthetic delete profile', 'synthetic-model', 100000, ?, ?, ?)",
    )
    .bind(profile_id.to_string())
    .bind(TIMESTAMP)
    .bind(TIMESTAMP)
    .bind(TIMESTAMP)
    .execute(pool)
    .await
    .unwrap();
    profile_id
}

async fn seed_full_book(
    pool: &SqlitePool,
    paths: &AppPaths,
    temporary: &tempfile::TempDir,
    profile_id: Uuid,
    label: &str,
    owned_bytes: &'static [u8],
    user_bytes: &'static [u8],
) -> BookSeed {
    let book_id = Uuid::new_v4();
    let section_id = Uuid::new_v4();
    let block_id = Uuid::new_v4();
    let chunk_id = Uuid::new_v4();
    let conversation_id = Uuid::new_v4();
    let book_conversation_id = Uuid::new_v4();
    let note_id = Uuid::new_v4();
    let annotation_id = Uuid::new_v4();
    let run_id = Uuid::new_v4();
    let page_id = Uuid::new_v4();
    let attempt_id = Uuid::new_v4();
    let index_block_id = Uuid::new_v4();
    let correction_id = Uuid::new_v4();
    let local_sentinel = format!("local{label}sentinel");
    let index_sentinel = format!("index{label}sentinel");
    let hash_character = if label == "target" { 'a' } else { 'b' };
    let source_hash = hash_character.to_string().repeat(64);
    let content_hash = if label == "target" { "c" } else { "d" }.repeat(64);
    let value_hash = if label == "target" { "e" } else { "f" }.repeat(64);
    let book_directory = paths.books.join(book_id.to_string());
    let derived = book_directory.join("derived");
    fs::create_dir_all(&derived).unwrap();
    let owned_source = book_directory.join("original.pdf");
    fs::write(&owned_source, owned_bytes).unwrap();
    fs::write(derived.join("document.html"), format!("<p>{label}</p>")).unwrap();
    let user_source = temporary.path().join(format!("user-{label}.pdf"));
    fs::write(&user_source, user_bytes).unwrap();

    sqlx::query(
        "INSERT INTO books (id, sha256, title, format, original_filename, stored_path, import_status, created_at, updated_at) VALUES (?, ?, ?, 'pdf', ?, ?, 'ready', ?, ?)",
    )
    .bind(book_id.to_string())
    .bind(source_hash)
    .bind(format!("Private {label} title"))
    .bind(format!("{label}.pdf"))
    .bind(format!("books/{book_id}/original.pdf"))
    .bind(TIMESTAMP)
    .bind(TIMESTAMP)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO sections (id, book_id, ordinal, title, locator_json) VALUES (?, ?, 0, ?, '{}')",
    )
    .bind(section_id.to_string())
    .bind(book_id.to_string())
    .bind(format!("{label} section"))
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO blocks (id, book_id, section_id, ordinal, kind, plain_text, locator_json) VALUES (?, ?, ?, 0, 'paragraph', ?, '{}')",
    )
    .bind(block_id.to_string())
    .bind(book_id.to_string())
    .bind(section_id.to_string())
    .bind(format!("{local_sentinel} body"))
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO search_chunks (id, book_id, section_id, ordinal, text, locator_json, token_estimate) VALUES (?, ?, ?, 0, ?, '{}', 4)",
    )
    .bind(chunk_id.to_string())
    .bind(book_id.to_string())
    .bind(section_id.to_string())
    .bind(format!("{local_sentinel} searchable"))
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO conversations (id, book_id, section_id, scope, anchor_kind, anchor_json, selected_text, created_at, updated_at) VALUES (?, ?, ?, 'selection', 'text', '{}', 'selected', ?, ?)",
    )
    .bind(conversation_id.to_string())
    .bind(book_id.to_string())
    .bind(section_id.to_string())
    .bind(TIMESTAMP)
    .bind(TIMESTAMP)
    .execute(pool)
    .await
    .unwrap();
    for (ordinal, role, content) in [
        (0_i64, "user", format!("{label} question")),
        (1_i64, "assistant", format!("{label} answer")),
    ] {
        if role == "user" {
            sqlx::query(
                "INSERT INTO messages (id, conversation_id, ordinal, role, action, content, created_at) VALUES (?, ?, ?, 'user', 'explain', ?, ?)",
            )
            .bind(Uuid::new_v4().to_string())
            .bind(conversation_id.to_string())
            .bind(ordinal)
            .bind(content)
            .bind(TIMESTAMP)
            .execute(pool)
            .await
            .unwrap();
        } else {
            sqlx::query(
                "INSERT INTO messages (id, conversation_id, ordinal, role, action, content, provider_id, model_id, citations_json, created_at) VALUES (?, ?, ?, 'assistant', 'explain', ?, ?, 'synthetic-model', '[]', ?)",
            )
            .bind(Uuid::new_v4().to_string())
            .bind(conversation_id.to_string())
            .bind(ordinal)
            .bind(content)
            .bind(profile_id.to_string())
            .bind(TIMESTAMP)
            .execute(pool)
            .await
            .unwrap();
        }
    }
    sqlx::query(
        "INSERT INTO annotations (id, book_id, section_id, kind, anchor_json, selected_text, conversation_id, revision, created_at, updated_at) VALUES (?, ?, ?, 'ai_conversation', '{}', 'selected', ?, 1, ?, ?)",
    )
    .bind(annotation_id.to_string())
    .bind(book_id.to_string())
    .bind(section_id.to_string())
    .bind(conversation_id.to_string())
    .bind(TIMESTAMP)
    .bind(TIMESTAMP)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO conversations (id, book_id, scope, created_at, updated_at) VALUES (?, ?, 'book', ?, ?)",
    )
    .bind(book_conversation_id.to_string())
    .bind(book_id.to_string())
    .bind(TIMESTAMP)
    .bind(TIMESTAMP)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO messages (id, conversation_id, ordinal, role, action, content, created_at) VALUES (?, ?, 0, 'user', 'ask', ?, ?)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(book_conversation_id.to_string())
    .bind(format!("{label} book question"))
    .bind(TIMESTAMP)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO messages (id, conversation_id, ordinal, role, action, content, provider_id, model_id, citations_json, created_at) VALUES (?, ?, 1, 'assistant', 'ask', ?, ?, 'synthetic-book-model', '[]', ?)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(book_conversation_id.to_string())
    .bind(format!("{label} book answer"))
    .bind(profile_id.to_string())
    .bind(TIMESTAMP)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO annotations (id, book_id, section_id, kind, anchor_json, selected_text, note_text, revision, created_at, updated_at) VALUES (?, ?, ?, 'note', '{}', 'selected', ?, 1, ?, ?)",
    )
    .bind(note_id.to_string())
    .bind(book_id.to_string())
    .bind(section_id.to_string())
    .bind(format!("{label} note"))
    .bind(TIMESTAMP)
    .bind(TIMESTAMP)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO index_runs (id, book_id, source_sha256, provider_profile_id, provider_kind, model_id, analysis_schema_version, render_version, parser_version, status, created_at, updated_at, completed_at) VALUES (?, ?, ?, ?, 'openai', 'synthetic-model', 'v1', 'v1', 'v1', 'completed', ?, ?, ?)",
    )
    .bind(run_id.to_string())
    .bind(book_id.to_string())
    .bind(hash_character.to_string().repeat(64))
    .bind(profile_id.to_string())
    .bind(TIMESTAMP)
    .bind(TIMESTAMP)
    .bind(TIMESTAMP)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO index_pages (id, run_id, book_id, page_number, quality_reason, status, attempt_id, attempt_count, content_sha256, content_version, created_at, updated_at) VALUES (?, ?, ?, 1, 'no_text', 'indexed', ?, 1, ?, 1, ?, ?)",
    )
    .bind(page_id.to_string())
    .bind(run_id.to_string())
    .bind(book_id.to_string())
    .bind(attempt_id.to_string())
    .bind(&content_hash)
    .bind(TIMESTAMP)
    .bind(TIMESTAMP)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO index_page_blocks (id, page_id, run_id, book_id, ordinal, kind, plain_text, source, provenance_version, content_version, value_sha256, created_at) VALUES (?, ?, ?, ?, 0, 'paragraph', ?, 'ai_transcribed', 1, 1, ?, ?)",
    )
    .bind(index_block_id.to_string())
    .bind(page_id.to_string())
    .bind(run_id.to_string())
    .bind(book_id.to_string())
    .bind(format!("{index_sentinel} indexed body"))
    .bind(&value_hash)
    .bind(TIMESTAMP)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO index_corrections (id, book_id, page_id, target_block_id, target_content_version, value_kind, original_value_sha256, original_value, corrected_value, conflict_state, revision, created_at, updated_at) VALUES (?, ?, ?, ?, 1, 'text', ?, 'original', 'corrected', 'active', 1, ?, ?)",
    )
    .bind(correction_id.to_string())
    .bind(book_id.to_string())
    .bind(page_id.to_string())
    .bind(index_block_id.to_string())
    .bind(&value_hash)
    .bind(TIMESTAMP)
    .bind(TIMESTAMP)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO index_search_chunks (id, book_id, page_id, block_id, ordinal, source, text, locator_json, token_estimate, content_version, created_at) VALUES (?, ?, ?, ?, 0, 'ai_transcribed', ?, '{}', 4, 1, ?)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(book_id.to_string())
    .bind(page_id.to_string())
    .bind(index_block_id.to_string())
    .bind(format!("{index_sentinel} searchable"))
    .bind(TIMESTAMP)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO index_search_chunks (id, book_id, page_id, block_id, correction_id, ordinal, source, text, locator_json, token_estimate, content_version, created_at) VALUES (?, ?, ?, ?, ?, 0, 'user_corrected', 'corrected searchable', '{}', 4, 1, ?)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(book_id.to_string())
    .bind(page_id.to_string())
    .bind(index_block_id.to_string())
    .bind(correction_id.to_string())
    .bind(TIMESTAMP)
    .execute(pool)
    .await
    .unwrap();

    let scratch_directory = paths.indexing_scratch().join(page_id.to_string());
    fs::create_dir_all(&scratch_directory).unwrap();
    let scratch_file = scratch_directory.join(format!("{attempt_id}.render"));
    let render_bytes: &[u8] = if label == "decoy" {
        b"decoy-render"
    } else {
        b"target-render"
    };
    fs::write(&scratch_file, render_bytes).unwrap();
    fs::write(
        scratch_directory.join(format!("{attempt_id}.tmp")),
        b"temporary-render",
    )
    .unwrap();

    BookSeed {
        book_id,
        run_id,
        page_id,
        user_source,
        user_bytes,
        book_directory,
        owned_source,
        owned_bytes,
        scratch_directory,
        scratch_file,
        local_sentinel,
        index_sentinel,
    }
}

async fn seed_minimal_book(
    pool: &SqlitePool,
    paths: &AppPaths,
    temporary: &tempfile::TempDir,
    ordinal: usize,
) -> BookSeed {
    let book_id = Uuid::new_v4();
    let run_id = Uuid::new_v4();
    let page_id = Uuid::new_v4();
    let attempt_id = Uuid::new_v4();
    let owned_bytes: &'static [u8] = b"crash-owned";
    let user_bytes: &'static [u8] = b"crash-user-original";
    let book_directory = paths.books.join(book_id.to_string());
    fs::create_dir_all(book_directory.join("derived")).unwrap();
    let owned_source = book_directory.join("original.pdf");
    fs::write(&owned_source, owned_bytes).unwrap();
    let user_source = temporary.path().join(format!("crash-user-{ordinal}.pdf"));
    fs::write(&user_source, user_bytes).unwrap();
    let source_hash = format!("{:064x}", ordinal + 1);
    sqlx::query(
        "INSERT INTO books (id, sha256, title, format, original_filename, stored_path, import_status, created_at, updated_at) VALUES (?, ?, 'Crash synthetic', 'pdf', 'crash.pdf', ?, 'ready', ?, ?)",
    )
    .bind(book_id.to_string())
    .bind(&source_hash)
    .bind(format!("books/{book_id}/original.pdf"))
    .bind(TIMESTAMP)
    .bind(TIMESTAMP)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO index_runs (id, book_id, source_sha256, provider_kind, model_id, analysis_schema_version, render_version, parser_version, status, created_at, updated_at, completed_at) VALUES (?, ?, ?, 'openai', 'model', 'v1', 'v1', 'v1', 'completed', ?, ?, ?)",
    )
    .bind(run_id.to_string())
    .bind(book_id.to_string())
    .bind(source_hash)
    .bind(TIMESTAMP)
    .bind(TIMESTAMP)
    .bind(TIMESTAMP)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO index_pages (id, run_id, book_id, page_number, quality_reason, status, attempt_id, attempt_count, content_sha256, content_version, created_at, updated_at) VALUES (?, ?, ?, 1, 'no_text', 'indexed', ?, 1, ?, 1, ?, ?)",
    )
    .bind(page_id.to_string())
    .bind(run_id.to_string())
    .bind(book_id.to_string())
    .bind(attempt_id.to_string())
    .bind("c".repeat(64))
    .bind(TIMESTAMP)
    .bind(TIMESTAMP)
    .execute(pool)
    .await
    .unwrap();
    let scratch_directory = paths.indexing_scratch().join(page_id.to_string());
    fs::create_dir_all(&scratch_directory).unwrap();
    let scratch_file = scratch_directory.join(format!("{attempt_id}.render"));
    fs::write(&scratch_file, b"crash-render").unwrap();
    BookSeed {
        book_id,
        run_id,
        page_id,
        user_source,
        user_bytes,
        book_directory,
        owned_source,
        owned_bytes,
        scratch_directory,
        scratch_file,
        local_sentinel: String::new(),
        index_sentinel: String::new(),
    }
}

async fn book_scoped_counts(pool: &SqlitePool, book_id: Uuid) -> HashMap<&'static str, i64> {
    let mut counts = HashMap::new();
    for (name, query) in [
        ("books", "SELECT COUNT(*) FROM books WHERE id = ?"),
        (
            "sections",
            "SELECT COUNT(*) FROM sections WHERE book_id = ?",
        ),
        ("blocks", "SELECT COUNT(*) FROM blocks WHERE book_id = ?"),
        (
            "search_chunks",
            "SELECT COUNT(*) FROM search_chunks WHERE book_id = ?",
        ),
        (
            "conversations",
            "SELECT COUNT(*) FROM conversations WHERE book_id = ?",
        ),
        (
            "annotations",
            "SELECT COUNT(*) FROM annotations WHERE book_id = ?",
        ),
        (
            "index_runs",
            "SELECT COUNT(*) FROM index_runs WHERE book_id = ?",
        ),
        (
            "index_pages",
            "SELECT COUNT(*) FROM index_pages WHERE book_id = ?",
        ),
        (
            "index_page_blocks",
            "SELECT COUNT(*) FROM index_page_blocks WHERE book_id = ?",
        ),
        (
            "index_corrections",
            "SELECT COUNT(*) FROM index_corrections WHERE book_id = ?",
        ),
        (
            "index_search_chunks",
            "SELECT COUNT(*) FROM index_search_chunks WHERE book_id = ?",
        ),
        (
            "remote_resources",
            "SELECT COUNT(*) FROM provider_remote_resources WHERE book_id = ?",
        ),
    ] {
        counts.insert(
            name,
            sqlx::query_scalar(query)
                .bind(book_id.to_string())
                .fetch_one(pool)
                .await
                .unwrap(),
        );
    }
    let messages: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM messages JOIN conversations ON conversations.id = messages.conversation_id WHERE conversations.book_id = ?",
    )
    .bind(book_id.to_string())
    .fetch_one(pool)
    .await
    .unwrap();
    counts.insert("messages", messages);
    counts
}

async fn book_exists(pool: &SqlitePool, book_id: Uuid) -> bool {
    sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM books WHERE id = ?)")
        .bind(book_id.to_string())
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn fts_match_count(pool: &SqlitePool, sentinel: &str) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM search_chunks_fts WHERE search_chunks_fts MATCH ?")
        .bind(sentinel)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn fts_index_match_count(pool: &SqlitePool, sentinel: &str) -> i64 {
    sqlx::query_scalar(
        "SELECT COUNT(*) FROM index_search_chunks_fts WHERE index_search_chunks_fts MATCH ?",
    )
    .bind(sentinel)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn cleanup_status(pool: &SqlitePool, resource_id: Uuid) -> String {
    sqlx::query_scalar("SELECT cleanup_status FROM provider_remote_resources WHERE id = ?")
        .bind(resource_id.to_string())
        .fetch_one(pool)
        .await
        .unwrap()
}

fn assert_no_pending_delete_artifacts(paths: &AppPaths) {
    let root = paths.cache.join("maintenance").join("delete-book");
    for name in ["intents", "trash"] {
        let directory = root.join(name);
        assert!(directory.is_dir());
        assert_eq!(fs::read_dir(directory).unwrap().count(), 0);
    }
}
