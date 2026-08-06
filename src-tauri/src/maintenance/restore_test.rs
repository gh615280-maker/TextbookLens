use std::{
    fs,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use sha2::{Digest, Sha256};
use sqlx::{Connection, sqlite::SqliteConnectOptions};
use tempfile::TempDir;

use super::*;
use crate::{
    credentials::MemoryCredentialStore,
    db::Database,
    maintenance::{archive::BackupService, storage::prepare_app_data_paths},
};

struct RestoreFixture {
    temporary: TempDir,
    paths: AppPaths,
    database: Database,
    gate: MaintenanceGate,
    archive: PathBuf,
    book_id: Uuid,
}

impl RestoreFixture {
    fn new() -> Self {
        let temporary = tempfile::tempdir().unwrap();
        let prepared = prepare_app_data_paths(
            &temporary
                .path()
                .join(crate::maintenance::storage::APP_DATA_DIRECTORY_NAME),
        )
        .unwrap();
        let paths = AppPaths {
            root: prepared.root,
            books: prepared.books,
            cache: prepared.cache,
            logs: prepared.logs,
            database: prepared.database,
        };
        let database = Database::open(&paths.database).unwrap();
        let book_id = uuid::uuid!("86c2f6dd-d617-4e32-9d2c-789f4a4eae42");
        let book = paths.books.join(book_id.to_string());
        fs::create_dir_all(book.join("derived")).unwrap();
        let source = b"%PDF-1.7\nRESTORE_SOURCE_SENTINEL\n%%EOF";
        fs::write(book.join("original.pdf"), source).unwrap();
        fs::write(
            book.join("derived/document.html"),
            b"<p>RESTORE_DERIVED_SENTINEL</p>",
        )
        .unwrap();
        tauri::async_runtime::block_on(async {
            sqlx::query("INSERT INTO books (id, sha256, title, format, original_filename, stored_path, import_status, created_at, updated_at) VALUES (?, ?, 'ARCHIVED_TITLE_SENTINEL', 'pdf', 'fixture.pdf', ?, 'ready', '2026-08-06T00:00:00.000Z', '2026-08-06T00:00:00.000Z')")
                .bind(book_id.to_string())
                .bind(hex(Sha256::digest(source)))
                .bind(format!("books/{book_id}/original.pdf"))
                .execute(database.pool())
                .await
                .unwrap();
            sqlx::query("INSERT INTO sections (id, book_id, ordinal, title, locator_json) VALUES (?, ?, 0, 'Section', '{}')")
                .bind(Uuid::new_v4().to_string())
                .bind(book_id.to_string())
                .execute(database.pool())
                .await
                .unwrap();
            sqlx::query("UPDATE app_settings SET ui_language = 'zh-TW' WHERE id = 1")
                .execute(database.pool())
                .await
                .unwrap();
        });
        let gate = MaintenanceGate::default();
        let archive = temporary.path().join("fixture.tlbackup");
        tauri::async_runtime::block_on(
            BackupService::new(database.pool().clone(), paths.clone(), gate.clone())
                .create_backup(archive.clone()),
        )
        .unwrap();
        Self {
            temporary,
            paths,
            database,
            gate,
            archive,
            book_id,
        }
    }

    fn service(&self) -> RestoreService {
        RestoreService::new(
            self.paths.clone(),
            self.gate.clone(),
            Arc::new(MemoryCredentialStore::new()),
        )
    }

    fn service_with_fault(&self, boundary: RestoreBoundary) -> RestoreService {
        RestoreService::with_fault_injector(
            self.paths.clone(),
            self.gate.clone(),
            Arc::new(MemoryCredentialStore::new()),
            Arc::new(FailOnceAt::new(boundary)),
        )
    }

    fn set_current_language(&self, language: &str) {
        tauri::async_runtime::block_on(
            sqlx::query("UPDATE app_settings SET ui_language = ? WHERE id = 1")
                .bind(language)
                .execute(self.database.pool()),
        )
        .unwrap();
    }
}

struct FailOnceAt {
    boundary: RestoreBoundary,
    fired: AtomicBool,
}

impl FailOnceAt {
    const fn new(boundary: RestoreBoundary) -> Self {
        Self {
            boundary,
            fired: AtomicBool::new(false),
        }
    }
}

impl RestoreFaultInjector for FailOnceAt {
    fn checkpoint(&self, boundary: RestoreBoundary) -> Result<(), RestoreInjectedCrash> {
        if boundary == self.boundary && !self.fired.swap(true, Ordering::SeqCst) {
            Err(RestoreInjectedCrash)
        } else {
            Ok(())
        }
    }
}

#[test]
fn restore_is_preflighted_then_atomically_installed_only_during_restart_recovery() {
    let fixture = RestoreFixture::new();
    tauri::async_runtime::block_on(async {
        sqlx::query("UPDATE app_settings SET ui_language = 'en' WHERE id = 1")
            .execute(fixture.database.pool())
            .await
            .unwrap();
    });

    let summary =
        tauri::async_runtime::block_on(fixture.service().stage_restore(fixture.archive.clone()))
            .unwrap();
    assert_eq!(summary.status, RestoreBackupStatusCode::ReadyToRestart);
    assert!(summary.restart_required);
    assert!(!summary.ai_configuration_required);
    assert_eq!(
        tauri::async_runtime::block_on(
            sqlx::query_scalar::<_, String>("SELECT ui_language FROM app_settings WHERE id = 1")
                .fetch_one(fixture.database.pool())
        )
        .unwrap(),
        "en"
    );

    tauri::async_runtime::block_on(fixture.database.pool().close());
    assert!(recover_pending_restore(&fixture.paths.root).unwrap());
    assert!(!recover_pending_restore(&fixture.paths.root).unwrap());
    let restored = Database::open(&fixture.paths.database).unwrap();
    let (language, title): (String, String) = tauri::async_runtime::block_on(async {
        (
            sqlx::query_scalar("SELECT ui_language FROM app_settings WHERE id = 1")
                .fetch_one(restored.pool())
                .await
                .unwrap(),
            sqlx::query_scalar("SELECT title FROM books WHERE id = ?")
                .bind(fixture.book_id.to_string())
                .fetch_one(restored.pool())
                .await
                .unwrap(),
        )
    });
    assert_eq!(language, "zh-TW");
    assert_eq!(title, "ARCHIVED_TITLE_SENTINEL");
    assert!(fixture.temporary.path().join("fixture.tlbackup").exists());
}

#[test]
fn busy_gate_rejects_restore_without_copying_or_cancelling_active_work() {
    let fixture = RestoreFixture::new();
    let permit = fixture
        .gate
        .try_acquire_normal(crate::domain::ActiveOperationKind::Learning)
        .unwrap();
    let error =
        tauri::async_runtime::block_on(fixture.service().stage_restore(fixture.archive.clone()))
            .unwrap_err();
    assert!(matches!(
        error,
        RestoreError::Gate(GateAcquireError::Busy(_))
    ));
    assert!(fixture.archive.exists());
    drop(permit);
}

#[test]
fn archive_corruption_and_noncanonical_ownership_fail_before_intent_or_mutation() {
    let fixture = RestoreFixture::new();
    let corrupt = fixture.temporary.path().join("corrupt.tlbackup");
    let mut bytes = fs::read(&fixture.archive).unwrap();
    bytes[16] ^= 0x40;
    fs::write(&corrupt, bytes).unwrap();
    let error =
        tauri::async_runtime::block_on(fixture.service().stage_restore(corrupt)).unwrap_err();
    assert_eq!(
        match error {
            RestoreError::Restore(error) => error.code,
            RestoreError::Gate(_) => panic!("unexpected gate error"),
        },
        MaintenanceErrorCode::RestoreArchiveInvalid
    );
    assert_eq!(
        tauri::async_runtime::block_on(
            sqlx::query_scalar::<_, String>("SELECT title FROM books WHERE id = ?")
                .bind(fixture.book_id.to_string())
                .fetch_one(fixture.database.pool())
        )
        .unwrap(),
        "ARCHIVED_TITLE_SENTINEL"
    );
}

#[test]
fn scheduling_faults_leave_no_orphan_or_a_recoverable_durable_intent() {
    for boundary in [
        RestoreBoundary::StageCreated,
        RestoreBoundary::ArchiveCopied,
        RestoreBoundary::ArchiveExtracted,
        RestoreBoundary::DatabasePreflightComplete,
        RestoreBoundary::IntentWritten,
    ] {
        let fixture = RestoreFixture::new();
        fixture.set_current_language("en");
        let error = tauri::async_runtime::block_on(
            fixture
                .service_with_fault(boundary)
                .stage_restore(fixture.archive.clone()),
        )
        .unwrap_err();
        assert!(matches!(error, RestoreError::Restore(_)));
        assert_eq!(current_language(fixture.database.pool()), "en");

        tauri::async_runtime::block_on(fixture.database.pool().close());
        if boundary == RestoreBoundary::IntentWritten {
            assert!(recover_pending_restore(&fixture.paths.root).unwrap());
            assert!(!recover_pending_restore(&fixture.paths.root).unwrap());
            assert_restored(&fixture);
        } else {
            assert!(!recover_pending_restore(&fixture.paths.root).unwrap());
            assert!(!recover_pending_restore(&fixture.paths.root).unwrap());
            let current = Database::open(&fixture.paths.database).unwrap();
            assert_eq!(current_language(current.pool()), "en");
        }
    }
}

#[test]
fn restart_fault_matrix_replays_to_one_complete_new_state() {
    let mut boundaries = Vec::new();
    for index in 0..4 {
        boundaries.push(RestoreBoundary::BeforeOldMove(index));
        boundaries.push(RestoreBoundary::AfterOldMove(index));
    }
    boundaries.push(RestoreBoundary::OldStateStaged);
    for index in 0..4 {
        boundaries.push(RestoreBoundary::BeforeNewMove(index));
        boundaries.push(RestoreBoundary::AfterNewMove(index));
    }
    boundaries.extend([
        RestoreBoundary::NewStateInstalled,
        RestoreBoundary::BeforeRollbackCleanup,
        RestoreBoundary::FinalizingIntentWritten,
        RestoreBoundary::BeforeStageCleanup,
        RestoreBoundary::BeforeIntentCleanup,
    ]);

    for boundary in boundaries {
        let fixture = RestoreFixture::new();
        fixture.set_current_language("en");
        tauri::async_runtime::block_on(fixture.service().stage_restore(fixture.archive.clone()))
            .unwrap();
        tauri::async_runtime::block_on(fixture.database.pool().close());

        let fault = FailOnceAt::new(boundary);
        assert!(recover_pending_restore_with_fault(&fixture.paths.root, &fault).is_err());
        assert!(recover_pending_restore(&fixture.paths.root).unwrap());
        assert!(!recover_pending_restore(&fixture.paths.root).unwrap());
        assert_restored(&fixture);
    }
}

#[test]
fn preflight_rejects_migration_drift_and_missing_owned_derived_file() {
    let fixture = RestoreFixture::new();
    let (operation, verified) = materialize_archive(&fixture);
    tauri::async_runtime::block_on(async {
        let options = SqliteConnectOptions::new()
            .filename(operation.dataset.join("library.sqlite3"))
            .create_if_missing(false);
        let mut connection = sqlx::SqliteConnection::connect_with(&options)
            .await
            .unwrap();
        sqlx::query(
            "DELETE FROM _sqlx_migrations WHERE version = (SELECT MAX(version) FROM _sqlx_migrations)",
        )
        .execute(&mut connection)
        .await
        .unwrap();
        connection.close().await.unwrap();
    });
    assert!(tauri::async_runtime::block_on(preflight_database(&operation, &verified)).is_err());

    let fixture = RestoreFixture::new();
    let (operation, mut verified) = materialize_archive(&fixture);
    verified
        .entries
        .retain(|entry| !entry.path.ends_with("/derived/document.html"));
    assert!(
        tauri::async_runtime::block_on(validate_archive_ownership(&operation.dataset, &verified,))
            .is_err()
    );
}

#[test]
fn hardlink_double_intent_and_root_decoy_fail_closed_before_data_loss() {
    let fixture = RestoreFixture::new();
    fixture.set_current_language("en");
    let alias = fixture.temporary.path().join("hardlink.tlbackup");
    fs::hard_link(&fixture.archive, &alias).unwrap();
    assert!(tauri::async_runtime::block_on(fixture.service().stage_restore(alias)).is_err());
    fs::remove_file(fixture.temporary.path().join("hardlink.tlbackup")).unwrap();

    tauri::async_runtime::block_on(fixture.service().stage_restore(fixture.archive.clone()))
        .unwrap();
    let store = RestoreStore::open_existing(&fixture.paths.root)
        .unwrap()
        .unwrap();
    let pending = store.list_intents().unwrap().pop().unwrap();
    let operation_decoy = store
        .operation(pending.intent.operation_id)
        .root
        .join("unexpected-inner");
    fs::create_dir(&operation_decoy).unwrap();
    tauri::async_runtime::block_on(fixture.database.pool().close());
    assert!(recover_pending_restore(&fixture.paths.root).is_err());
    assert!(fixture.paths.database.exists());
    fs::remove_dir(operation_decoy).unwrap();

    let control_decoy = store.staging.parent().unwrap().join("unexpected-control");
    fs::create_dir(&control_decoy).unwrap();
    assert!(recover_pending_restore(&fixture.paths.root).is_err());
    assert!(fixture.paths.database.exists());
    fs::remove_dir(control_decoy).unwrap();

    let duplicate = store.old_staged.join(
        pending
            .intent_path
            .file_name()
            .expect("intent has a canonical file name"),
    );
    fs::copy(&pending.intent_path, &duplicate).unwrap();
    assert!(recover_pending_restore(&fixture.paths.root).is_err());
    assert!(fixture.paths.database.exists());
    fs::remove_file(duplicate).unwrap();

    let original_intent = fs::read(&pending.intent_path).unwrap();
    let mut altered_intent: serde_json::Value = serde_json::from_slice(&original_intent).unwrap();
    altered_intent["manifestSha256"] = serde_json::Value::String("0".repeat(64));
    fs::write(
        &pending.intent_path,
        serde_json::to_vec(&altered_intent).unwrap(),
    )
    .unwrap();
    assert!(recover_pending_restore(&fixture.paths.root).is_err());
    assert!(fixture.paths.database.exists());
    fs::write(&pending.intent_path, original_intent).unwrap();

    let decoy = fixture.paths.root.join("unexpected.bin");
    fs::write(&decoy, b"DECOY_MUST_SURVIVE").unwrap();
    assert!(recover_pending_restore(&fixture.paths.root).is_err());
    assert_eq!(fs::read(&decoy).unwrap(), b"DECOY_MUST_SURVIVE");
    assert!(fixture.paths.database.exists());
    fs::remove_file(decoy).unwrap();
    assert!(recover_pending_restore(&fixture.paths.root).unwrap());
    assert_restored(&fixture);
}

#[test]
fn restore_debug_surfaces_are_structural_and_redacted() {
    let fixture = RestoreFixture::new();
    let service = fixture.service();
    let rendered = format!(
        "{service:?} {:?}",
        RestoreArchiveError::new(MaintenanceErrorCode::RestorePreflightFailed)
    );
    assert!(!rendered.contains(fixture.paths.root.to_string_lossy().as_ref()));
    assert!(!rendered.contains("ARCHIVED_TITLE_SENTINEL"));
    assert!(!rendered.contains("RESTORE_SOURCE_SENTINEL"));
}

fn materialize_archive(fixture: &RestoreFixture) -> (RestoreOperationPaths, VerifiedBackup) {
    let store = RestoreStore::open_or_create(&fixture.paths.root).unwrap();
    let operation = store.create_stage(Uuid::new_v4()).unwrap();
    let verified =
        copy_archive_into_staging(&fixture.archive, &operation.archive, &fixture.paths.root)
            .unwrap();
    extract_verified_archive(&operation.archive, &verified, &operation.dataset).unwrap();
    prepare_empty_runtime_directories(&operation.dataset).unwrap();
    validate_materialized_dataset(&operation.dataset, &verified).unwrap();
    (operation, verified)
}

fn current_language(pool: &sqlx::SqlitePool) -> String {
    tauri::async_runtime::block_on(
        sqlx::query_scalar("SELECT ui_language FROM app_settings WHERE id = 1").fetch_one(pool),
    )
    .unwrap()
}

fn assert_restored(fixture: &RestoreFixture) {
    let restored = Database::open(&fixture.paths.database).unwrap();
    assert_eq!(current_language(restored.pool()), "zh-TW");
    assert_eq!(
        tauri::async_runtime::block_on(
            sqlx::query_scalar::<_, String>("SELECT title FROM books WHERE id = ?")
                .bind(fixture.book_id.to_string())
                .fetch_one(restored.pool()),
        )
        .unwrap(),
        "ARCHIVED_TITLE_SENTINEL"
    );
    tauri::async_runtime::block_on(restored.pool().close());
    assert!(fixture.archive.exists());
}

fn hex(bytes: impl AsRef<[u8]>) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::new();
    for byte in bytes.as_ref() {
        output.push(char::from(DIGITS[usize::from(byte >> 4)]));
        output.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    output
}
