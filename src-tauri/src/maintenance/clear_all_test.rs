use std::{
    collections::HashMap,
    fs,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use async_trait::async_trait;
use secrecy::SecretString;
use tempfile::TempDir;

use super::*;
use crate::{
    credentials::CredentialStore,
    db::Database,
    maintenance::storage::{APP_DATA_DIRECTORY_NAME, prepare_app_data_paths},
};

#[derive(Default)]
struct FakeCredentialStore {
    values: Mutex<HashMap<String, SecretString>>,
    fail_deletes: Mutex<usize>,
}

impl FakeCredentialStore {
    fn insert(&self, key: String) {
        self.values
            .lock()
            .unwrap()
            .insert(key, SecretString::from("SYNTHETIC_SECRET_SENTINEL"));
    }

    fn fail_once(&self) {
        *self.fail_deletes.lock().unwrap() = 1;
    }

    fn is_empty(&self) -> bool {
        self.values.lock().unwrap().is_empty()
    }
}

impl std::fmt::Debug for FakeCredentialStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("FakeCredentialStore(<redacted>)")
    }
}

#[async_trait]
impl CredentialStore for FakeCredentialStore {
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
        let mut failures = self.fail_deletes.lock().unwrap();
        if *failures > 0 {
            *failures -= 1;
            return Err(AppError::new(AppErrorCode::CredentialStoreError));
        }
        self.values.lock().unwrap().remove(key);
        Ok(())
    }

    async fn list_textbooklens_keys(&self) -> AppResult<Vec<String>> {
        Ok(self.values.lock().unwrap().keys().cloned().collect())
    }
}

struct ClearFixture {
    temporary: TempDir,
    paths: AppPaths,
    database: Database,
    gate: MaintenanceGate,
    credentials: Arc<FakeCredentialStore>,
    external_backup: std::path::PathBuf,
    original_source: std::path::PathBuf,
}

impl ClearFixture {
    fn new() -> Self {
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
        let profile_id = uuid::uuid!("0b101dcb-47af-4e1d-9aa3-36472bf148a6");
        let remote_resource_id = uuid::uuid!("0b101dcb-47af-4e1d-9aa3-36472bf148a7");
        tauri::async_runtime::block_on(async {
            sqlx::query("INSERT INTO provider_profiles (id, provider_kind, display_name, model_id, context_window_tokens, is_active, created_at, updated_at, validated_at) VALUES (?, 'openai', 'Fixture', 'fixture-model', 4096, 1, '2026-08-06T00:00:00.000Z', '2026-08-06T00:00:00.000Z', '2026-08-06T00:00:00.000Z')")
                .bind(profile_id.to_string())
                .execute(database.pool())
                .await
                .unwrap();
            sqlx::query("INSERT INTO provider_remote_resources (id, provider_kind, encrypted_reference, cleanup_status, created_at, updated_at) VALUES (?, 'openai', ?, 'pending', '2026-08-06T00:00:00.000Z', '2026-08-06T00:00:00.000Z')")
                .bind(remote_resource_id.to_string())
                .bind(format!("enc:v1:keyring:{remote_resource_id}"))
                .execute(database.pool())
                .await
                .unwrap();
        });
        fs::write(paths.books.join("owned-data.bin"), b"OWNED_DATA_SENTINEL").unwrap();
        fs::write(paths.cache.join("cache.bin"), b"CACHE_SENTINEL").unwrap();
        fs::write(paths.logs.join("log.txt"), b"LOG_SENTINEL").unwrap();
        let external_backup = temporary.path().join("external.tlbackup");
        let original_source = temporary.path().join("original.pdf");
        fs::write(&external_backup, b"EXTERNAL_BACKUP_SENTINEL").unwrap();
        fs::write(&original_source, b"ORIGINAL_SOURCE_SENTINEL").unwrap();
        let credentials = Arc::new(FakeCredentialStore::default());
        credentials.insert(credential_key(profile_id));
        credentials.insert(format!("textbooklens/remote-resource/{remote_resource_id}"));
        Self {
            temporary,
            paths,
            database,
            gate: MaintenanceGate::default(),
            credentials,
            external_backup,
            original_source,
        }
    }

    fn service(&self) -> ClearAllDataService {
        ClearAllDataService::new(
            self.database.pool().clone(),
            self.paths.clone(),
            self.gate.clone(),
            self.credentials.clone(),
        )
    }

    fn service_with_fault(&self, boundary: ClearBoundary) -> ClearAllDataService {
        ClearAllDataService::with_fault_injector(
            self.database.pool().clone(),
            self.paths.clone(),
            self.gate.clone(),
            self.credentials.clone(),
            Arc::new(FailOnceAt::new(boundary)),
        )
    }
}

struct FailOnceAt {
    boundary: ClearBoundary,
    fired: AtomicBool,
}

impl FailOnceAt {
    const fn new(boundary: ClearBoundary) -> Self {
        Self {
            boundary,
            fired: AtomicBool::new(false),
        }
    }
}

impl ClearFaultInjector for FailOnceAt {
    fn checkpoint(&self, boundary: ClearBoundary) -> Result<(), ClearInjectedCrash> {
        if boundary == self.boundary && !self.fired.swap(true, Ordering::SeqCst) {
            Err(ClearInjectedCrash)
        } else {
            Ok(())
        }
    }
}

#[test]
fn confirmation_is_independent_exact_and_checked_before_any_mutation() {
    let fixture = ClearFixture::new();
    for invalid in [
        "",
        "delete all textbooklens data",
        "DELETE ALL TEXTBOOKLENS DATA ",
    ] {
        let error =
            tauri::async_runtime::block_on(fixture.service().clear_all_data(invalid.to_owned()))
                .unwrap_err();
        assert_eq!(
            match error {
                ClearServiceError::Clear(error) => error.code,
                ClearServiceError::Gate(_) => panic!("unexpected gate error"),
            },
            MaintenanceErrorCode::ClearConfirmationRequired
        );
    }
    assert!(!fixture.credentials.is_empty());
    assert!(fixture.paths.database.exists());
}

#[test]
fn credential_failure_is_observable_retryable_and_never_reports_complete() {
    let fixture = ClearFixture::new();
    fixture.credentials.fail_once();
    let first = tauri::async_runtime::block_on(
        fixture
            .service()
            .clear_all_data(CLEAR_ALL_DATA_CONFIRMATION_PHRASE.to_owned()),
    )
    .unwrap();
    assert_eq!(
        first.status,
        ClearAllDataStatusCode::CredentialCleanupRequired
    );
    assert!(!first.restart_required);
    assert!(fixture.paths.database.exists());

    let retried = tauri::async_runtime::block_on(
        fixture
            .service()
            .clear_all_data(CLEAR_ALL_DATA_CONFIRMATION_PHRASE.to_owned()),
    )
    .unwrap();
    assert_eq!(retried.status, ClearAllDataStatusCode::ReadyToRestart);
    assert!(retried.restart_required);
    assert!(fixture.credentials.is_empty());
}

#[test]
fn restart_clear_is_idempotent_and_preserves_external_backup_and_original() {
    let fixture = ClearFixture::new();
    let summary = tauri::async_runtime::block_on(
        fixture
            .service()
            .clear_all_data(CLEAR_ALL_DATA_CONFIRMATION_PHRASE.to_owned()),
    )
    .unwrap();
    assert_eq!(summary.status, ClearAllDataStatusCode::ReadyToRestart);
    tauri::async_runtime::block_on(fixture.database.pool().close());
    let outcome = tauri::async_runtime::block_on(recover_pending_clear(
        &fixture.paths.root,
        fixture.credentials.as_ref(),
    ))
    .unwrap();
    assert_eq!(outcome, ClearStartupOutcome::Cleared);
    assert!(!fixture.paths.database.exists());
    assert!(!fixture.paths.books.exists());
    assert_eq!(
        fs::read(&fixture.external_backup).unwrap(),
        b"EXTERNAL_BACKUP_SENTINEL"
    );
    assert_eq!(
        fs::read(&fixture.original_source).unwrap(),
        b"ORIGINAL_SOURCE_SENTINEL"
    );
    let _ = &fixture.temporary;
}

#[test]
fn active_work_rejects_clear_without_cancellation_or_credential_mutation() {
    let fixture = ClearFixture::new();
    let permit = fixture
        .gate
        .try_acquire_normal(crate::domain::ActiveOperationKind::Import)
        .unwrap();
    let error = tauri::async_runtime::block_on(
        fixture
            .service()
            .clear_all_data(CLEAR_ALL_DATA_CONFIRMATION_PHRASE.to_owned()),
    )
    .unwrap_err();
    assert!(matches!(
        error,
        ClearServiceError::Gate(GateAcquireError::Busy(_))
    ));
    assert!(!fixture.credentials.is_empty());
    assert!(fixture.paths.database.exists());
    drop(permit);
}

#[test]
fn credential_and_intent_faults_are_durable_and_restart_retryable() {
    for boundary in [
        ClearBoundary::IntentWritten,
        ClearBoundary::BeforeCredentialDelete(0),
        ClearBoundary::AfterCredentialDelete(0),
        ClearBoundary::CredentialsCleared,
    ] {
        let fixture = ClearFixture::new();
        let error = tauri::async_runtime::block_on(
            fixture
                .service_with_fault(boundary)
                .clear_all_data(CLEAR_ALL_DATA_CONFIRMATION_PHRASE.to_owned()),
        )
        .unwrap_err();
        assert!(matches!(error, ClearServiceError::Clear(_)));
        assert!(fixture.paths.database.exists());
        tauri::async_runtime::block_on(fixture.database.pool().close());
        assert_eq!(
            tauri::async_runtime::block_on(recover_pending_clear(
                &fixture.paths.root,
                fixture.credentials.as_ref(),
            ))
            .unwrap(),
            ClearStartupOutcome::Cleared
        );
        assert_eq!(
            tauri::async_runtime::block_on(recover_pending_clear(
                &fixture.paths.root,
                fixture.credentials.as_ref(),
            ))
            .unwrap(),
            ClearStartupOutcome::NoPending
        );
        assert!(!fixture.paths.database.exists());
        assert!(fixture.credentials.is_empty());
    }
}

#[test]
fn local_clear_fault_matrix_replays_idempotently_to_first_start_state() {
    let mut boundaries = Vec::new();
    for index in 0..4 {
        boundaries.push(ClearBoundary::BeforeLocalMove(index));
        boundaries.push(ClearBoundary::AfterLocalMove(index));
    }
    boundaries.extend([
        ClearBoundary::LocalStateStaged,
        ClearBoundary::BeforeTrashCleanup,
        ClearBoundary::BeforeIntentCleanup,
    ]);

    for boundary in boundaries {
        let fixture = ClearFixture::new();
        tauri::async_runtime::block_on(
            fixture
                .service()
                .clear_all_data(CLEAR_ALL_DATA_CONFIRMATION_PHRASE.to_owned()),
        )
        .unwrap();
        tauri::async_runtime::block_on(fixture.database.pool().close());
        let fault = FailOnceAt::new(boundary);
        assert!(
            tauri::async_runtime::block_on(recover_pending_clear_with_fault(
                &fixture.paths.root,
                fixture.credentials.as_ref(),
                &fault,
            ))
            .is_err()
        );
        assert_eq!(
            tauri::async_runtime::block_on(recover_pending_clear(
                &fixture.paths.root,
                fixture.credentials.as_ref(),
            ))
            .unwrap(),
            ClearStartupOutcome::Cleared
        );
        assert_eq!(
            tauri::async_runtime::block_on(recover_pending_clear(
                &fixture.paths.root,
                fixture.credentials.as_ref(),
            ))
            .unwrap(),
            ClearStartupOutcome::NoPending
        );
        assert_external_files_preserved(&fixture);
    }
}

#[test]
fn unknown_root_and_hardlink_alias_fail_closed_and_are_retryable() {
    let fixture = ClearFixture::new();
    tauri::async_runtime::block_on(
        fixture
            .service()
            .clear_all_data(CLEAR_ALL_DATA_CONFIRMATION_PHRASE.to_owned()),
    )
    .unwrap();
    tauri::async_runtime::block_on(fixture.database.pool().close());
    let control_decoy = fixture
        .paths
        .root
        .join(MAINTENANCE_DIRECTORY)
        .join(CLEAR_DIRECTORY)
        .join("unexpected-control");
    fs::create_dir(&control_decoy).unwrap();
    assert!(
        tauri::async_runtime::block_on(recover_pending_clear(
            &fixture.paths.root,
            fixture.credentials.as_ref(),
        ))
        .is_err()
    );
    assert!(fixture.paths.database.exists());
    fs::remove_dir(control_decoy).unwrap();

    let root_decoy = fixture.paths.root.join("unknown.bin");
    fs::write(&root_decoy, b"ROOT_DECOY_SENTINEL").unwrap();
    assert!(
        tauri::async_runtime::block_on(recover_pending_clear(
            &fixture.paths.root,
            fixture.credentials.as_ref(),
        ))
        .is_err()
    );
    assert_eq!(fs::read(&root_decoy).unwrap(), b"ROOT_DECOY_SENTINEL");
    assert!(fixture.paths.database.exists());
    fs::remove_file(root_decoy).unwrap();

    let outside_alias = fixture.temporary.path().join("outside-hardlink.bin");
    fs::hard_link(fixture.paths.books.join("owned-data.bin"), &outside_alias).unwrap();
    assert!(
        tauri::async_runtime::block_on(recover_pending_clear(
            &fixture.paths.root,
            fixture.credentials.as_ref(),
        ))
        .is_err()
    );
    assert_eq!(fs::read(&outside_alias).unwrap(), b"OWNED_DATA_SENTINEL");
    let store = ClearStore::open_existing(&fixture.paths.root)
        .unwrap()
        .unwrap();
    let pending = store.single_intent().unwrap().unwrap();
    let trash_decoy = store
        .trash_root(pending.intent.operation_id)
        .join("unexpected-inner");
    fs::create_dir(&trash_decoy).unwrap();
    assert!(
        tauri::async_runtime::block_on(recover_pending_clear(
            &fixture.paths.root,
            fixture.credentials.as_ref(),
        ))
        .is_err()
    );
    assert_eq!(fs::read(&outside_alias).unwrap(), b"OWNED_DATA_SENTINEL");
    fs::remove_dir(trash_decoy).unwrap();
    fs::remove_file(outside_alias).unwrap();
    assert_eq!(
        tauri::async_runtime::block_on(recover_pending_clear(
            &fixture.paths.root,
            fixture.credentials.as_ref(),
        ))
        .unwrap(),
        ClearStartupOutcome::Cleared
    );
    assert_external_files_preserved(&fixture);
}

#[cfg(windows)]
#[test]
fn root_reparse_directory_is_rejected_without_following_external_data() {
    let fixture = ClearFixture::new();
    tauri::async_runtime::block_on(
        fixture
            .service()
            .clear_all_data(CLEAR_ALL_DATA_CONFIRMATION_PHRASE.to_owned()),
    )
    .unwrap();
    tauri::async_runtime::block_on(fixture.database.pool().close());
    let outside = fixture.temporary.path().join("outside-cache");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("decoy.bin"), b"OUTSIDE_REPARSE_SENTINEL").unwrap();
    fs::remove_dir_all(&fixture.paths.cache).unwrap();
    create_windows_junction(&outside, &fixture.paths.cache);

    assert!(
        tauri::async_runtime::block_on(recover_pending_clear(
            &fixture.paths.root,
            fixture.credentials.as_ref(),
        ))
        .is_err()
    );
    assert_eq!(
        fs::read(outside.join("decoy.bin")).unwrap(),
        b"OUTSIDE_REPARSE_SENTINEL"
    );
    fs::remove_dir(&fixture.paths.cache).unwrap();
    fs::create_dir(&fixture.paths.cache).unwrap();
    assert_eq!(
        tauri::async_runtime::block_on(recover_pending_clear(
            &fixture.paths.root,
            fixture.credentials.as_ref(),
        ))
        .unwrap(),
        ClearStartupOutcome::Cleared
    );
    assert_eq!(
        fs::read(outside.join("decoy.bin")).unwrap(),
        b"OUTSIDE_REPARSE_SENTINEL"
    );
}

#[test]
fn clear_debug_surfaces_redact_paths_credentials_and_internal_ids() {
    let fixture = ClearFixture::new();
    let id = uuid::uuid!("0b101dcb-47af-4e1d-9aa3-36472bf148a6");
    let intent = ClearIntent::new(
        Uuid::new_v4(),
        vec![CredentialTarget::new(credential_key(id)).unwrap()],
    )
    .unwrap();
    let rendered = format!("{:?} {:?}", fixture.service(), intent);
    assert!(!rendered.contains(fixture.paths.root.to_string_lossy().as_ref()));
    assert!(!rendered.contains(&id.to_string()));
    assert!(!rendered.contains("SYNTHETIC_SECRET_SENTINEL"));
}

fn assert_external_files_preserved(fixture: &ClearFixture) {
    assert_eq!(
        fs::read(&fixture.external_backup).unwrap(),
        b"EXTERNAL_BACKUP_SENTINEL"
    );
    assert_eq!(
        fs::read(&fixture.original_source).unwrap(),
        b"ORIGINAL_SOURCE_SENTINEL"
    );
}

#[cfg(windows)]
fn create_windows_junction(source: &std::path::Path, target: &std::path::Path) {
    let output = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(target)
        .arg(source)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "junction creation failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
