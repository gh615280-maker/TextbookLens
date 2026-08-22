use std::{
    collections::{HashMap, HashSet},
    fmt, fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
};

use serde::{Deserialize, Serialize};
use sqlx::{Connection, Row, SqliteConnection, sqlite::SqliteConnectOptions};
use uuid::Uuid;

use crate::{
    app_state::AppPaths,
    credentials::CredentialStore,
    db::{MIGRATOR, providers::credential_key},
    domain::{
        MaintenanceErrorCode, MaintenanceErrorDto, RestoreBackupStatusCode, RestoreBackupSummaryDto,
    },
    errors::{AppError, AppErrorCode, AppResult},
    maintenance::{
        archive::{
            BackupArchiveError, VerifiedBackup, copy_archive_into_staging,
            extract_verified_archive, require_safe_directory, safe_path_exists, sync_app_directory,
            verify_archive_file, verify_materialized_file,
        },
        gate::{GateAcquireError, MaintenanceGate},
        journal::metadata_is_reparse,
        recovery_fs::{
            OwnedEntryKind, capture_tree, ensure_direct_child_directory, path_exists,
            remove_captured_tree, remove_tree, rename_checked, require_app_root,
            require_directory_children, require_exact_child_directory, require_regular_file, sync,
        },
        storage::StorageLayout,
    },
};

const RESTORE_INTENT_FORMAT: &str = "textbooklens.restore-intent";
const RESTORE_INTENT_VERSION: u32 = 1;
const MAINTENANCE_DIRECTORY: &str = "maintenance";
const RESTORE_DIRECTORY: &str = "restore";
const INTENTS_DIRECTORY: &str = "intents";
const PREPARED_DIRECTORY: &str = "prepared";
const OLD_STAGED_DIRECTORY: &str = "old-staged";
const NEW_INSTALLED_DIRECTORY: &str = "new-installed";
const FINALIZING_DIRECTORY: &str = "finalizing";
const STAGING_DIRECTORY: &str = "staging";
const ROLLBACK_DIRECTORY: &str = "rollback";
const STAGED_ARCHIVE: &str = "archive.tlbackup";
const STAGED_DATASET: &str = "dataset";
const MAX_INTENT_BYTES: u64 = 8 * 1024;
const MAX_CONTROL_ENTRIES: usize = 64;
const MAX_BOOK_ROWS: usize = 100_000;
const SQLITE_SIDECARS: [&str; 3] = [
    "library.sqlite3-wal",
    "library.sqlite3-shm",
    "library.sqlite3-journal",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RestoreBoundary {
    StageCreated,
    ArchiveCopied,
    ArchiveExtracted,
    DatabasePreflightComplete,
    IntentWritten,
    BeforeOldMove(usize),
    AfterOldMove(usize),
    OldStateStaged,
    BeforeNewMove(usize),
    AfterNewMove(usize),
    NewStateInstalled,
    BeforeRollbackCleanup,
    FinalizingIntentWritten,
    BeforeStageCleanup,
    BeforeIntentCleanup,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RestoreInjectedCrash;

pub trait RestoreFaultInjector: Send + Sync {
    fn checkpoint(&self, boundary: RestoreBoundary) -> Result<(), RestoreInjectedCrash>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct NoRestoreFault;

impl RestoreFaultInjector for NoRestoreFault {
    fn checkpoint(&self, _boundary: RestoreBoundary) -> Result<(), RestoreInjectedCrash> {
        Ok(())
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct RestoreArchiveError {
    pub code: MaintenanceErrorCode,
}

impl RestoreArchiveError {
    const fn new(code: MaintenanceErrorCode) -> Self {
        Self { code }
    }
}

impl fmt::Debug for RestoreArchiveError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RestoreArchiveError")
            .field("code", &self.code)
            .finish()
    }
}

impl fmt::Display for RestoreArchiveError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:?}", self.code)
    }
}

impl std::error::Error for RestoreArchiveError {}

#[derive(PartialEq, Eq)]
pub enum RestoreError {
    Gate(GateAcquireError),
    Restore(RestoreArchiveError),
}

impl fmt::Debug for RestoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Gate(error) => formatter.debug_tuple("Gate").field(error).finish(),
            Self::Restore(error) => formatter.debug_tuple("Restore").field(error).finish(),
        }
    }
}

impl fmt::Display for RestoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Gate(_) => formatter.write_str("restore maintenance gate unavailable"),
            Self::Restore(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for RestoreError {}

impl From<GateAcquireError> for RestoreError {
    fn from(error: GateAcquireError) -> Self {
        Self::Gate(error)
    }
}

impl From<RestoreArchiveError> for RestoreError {
    fn from(error: RestoreArchiveError) -> Self {
        Self::Restore(error)
    }
}

impl From<RestoreError> for MaintenanceErrorDto {
    fn from(error: RestoreError) -> Self {
        match error {
            RestoreError::Gate(error) => Self::from(error),
            RestoreError::Restore(error) => Self::new(error.code),
        }
    }
}

#[derive(Clone)]
pub struct RestoreService {
    paths: AppPaths,
    gate: MaintenanceGate,
    credentials: Arc<dyn CredentialStore>,
    fault: Arc<dyn RestoreFaultInjector>,
}

impl RestoreService {
    pub fn new(
        paths: AppPaths,
        gate: MaintenanceGate,
        credentials: Arc<dyn CredentialStore>,
    ) -> Self {
        Self {
            paths,
            gate,
            credentials,
            fault: Arc::new(NoRestoreFault),
        }
    }

    #[doc(hidden)]
    pub fn with_fault_injector(
        paths: AppPaths,
        gate: MaintenanceGate,
        credentials: Arc<dyn CredentialStore>,
        fault: Arc<dyn RestoreFaultInjector>,
    ) -> Self {
        Self {
            paths,
            gate,
            credentials,
            fault,
        }
    }

    pub async fn stage_restore(
        self,
        archive: PathBuf,
    ) -> Result<RestoreBackupSummaryDto, RestoreError> {
        let permit = self.gate.try_acquire_maintenance()?;
        StorageLayout::from_app_paths(&self.paths).map_err(|_| restore_source_invalid())?;
        let root = require_app_root(&self.paths.root).map_err(|_| restore_source_invalid())?;
        validate_restore_source_path(&archive, &root)?;
        let store = RestoreStore::open_or_create(&root)?;
        store.require_no_pending_or_conflicting_intent()?;
        let operation_id = Uuid::new_v4();
        let operation = store.create_stage(operation_id)?;

        let result: Result<RestoreBackupSummaryDto, RestoreArchiveError> = async {
            checkpoint(self.fault.as_ref(), RestoreBoundary::StageCreated)?;
            let archive_path = archive;
            let staged_archive = operation.archive.clone();
            let dataset = operation.dataset.clone();
            let app_root = root.clone();
            let verified = tokio::task::spawn_blocking(move || {
                let verified =
                    copy_archive_into_staging(&archive_path, &staged_archive, &app_root)?;
                Ok::<VerifiedBackup, BackupArchiveError>(verified)
            })
            .await
            .map_err(|_| restore_stage_failed())?
            .map_err(map_archive_source_error)?;
            checkpoint(self.fault.as_ref(), RestoreBoundary::ArchiveCopied)?;

            let staged_archive = operation.archive.clone();
            let dataset = dataset.clone();
            let verified = Arc::new(verified);
            let extract_verified = verified.clone();
            tokio::task::spawn_blocking(move || {
                extract_verified_archive(&staged_archive, &extract_verified, &dataset)
            })
            .await
            .map_err(|_| restore_stage_failed())?
            .map_err(|_| restore_archive_invalid())?;
            checkpoint(self.fault.as_ref(), RestoreBoundary::ArchiveExtracted)?;

            prepare_empty_runtime_directories(&operation.dataset)?;
            validate_materialized_dataset(&operation.dataset, &verified)?;
            let preflight_operation = operation.clone();
            let preflight_verified = verified.clone();
            let profiles = tokio::task::spawn_blocking(move || {
                tauri::async_runtime::block_on(async {
                    let profiles =
                        preflight_database(&preflight_operation, &preflight_verified).await?;
                    validate_archive_ownership(&preflight_operation.dataset, &preflight_verified)
                        .await?;
                    Ok::<Vec<Uuid>, RestoreArchiveError>(profiles)
                })
            })
            .await
            .map_err(|_| restore_preflight_failed())??;
            checkpoint(
                self.fault.as_ref(),
                RestoreBoundary::DatabasePreflightComplete,
            )?;

            let mut ai_configuration_required = false;
            for profile_id in profiles {
                if self
                    .credentials
                    .get(&credential_key(profile_id))
                    .await
                    .is_err()
                {
                    ai_configuration_required = true;
                }
            }
            let intent = RestoreIntent::new(operation_id, &verified);
            store.write_prepared_intent(&intent)?;
            checkpoint(self.fault.as_ref(), RestoreBoundary::IntentWritten)?;
            self.gate.shutdown();
            Ok(RestoreBackupSummaryDto {
                status: RestoreBackupStatusCode::ReadyToRestart,
                restart_required: true,
                ai_configuration_required,
            })
        }
        .await;

        if result.is_err() && !store.has_intent(operation_id)? {
            store.remove_uncommitted_stage(&operation)?;
        }
        drop(permit);
        result.map_err(RestoreError::from)
    }
}

impl fmt::Debug for RestoreService {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("RestoreService(<redacted>)")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RestorePhase {
    Prepared,
    OldStaged,
    NewInstalled,
    Finalizing,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct RestoreIntent {
    format: String,
    version: u32,
    operation_id: Uuid,
    archive_bytes: u64,
    entry_count: u64,
    payload_bytes: u64,
    manifest_sha256: String,
}

impl RestoreIntent {
    fn new(operation_id: Uuid, verified: &VerifiedBackup) -> Self {
        Self {
            format: RESTORE_INTENT_FORMAT.to_owned(),
            version: RESTORE_INTENT_VERSION,
            operation_id,
            archive_bytes: verified.archive_bytes,
            entry_count: verified.entry_count,
            payload_bytes: verified.total_bytes,
            manifest_sha256: verified.manifest_sha256.clone(),
        }
    }

    fn validate(&self, expected_id: Uuid) -> Result<(), RestoreArchiveError> {
        if self.format != RESTORE_INTENT_FORMAT
            || self.version != RESTORE_INTENT_VERSION
            || self.operation_id != expected_id
            || self.operation_id.is_nil()
            || self.archive_bytes == 0
            || self.entry_count == 0
            || self.entry_count > 100_000
            || self.payload_bytes == 0
            || self.payload_bytes > (1_u64 << 40)
            || self.manifest_sha256.len() != 64
            || !self
                .manifest_sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
        {
            return Err(restore_intent_conflict());
        }
        Ok(())
    }
}

impl fmt::Debug for RestoreIntent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RestoreIntent")
            .field("operation_id", &"<redacted>")
            .field("archive_bytes", &self.archive_bytes)
            .field("entry_count", &self.entry_count)
            .field("payload_bytes", &self.payload_bytes)
            .field("manifest_sha256", &"<redacted>")
            .finish()
    }
}

#[derive(Clone)]
struct RestoreOperationPaths {
    root: PathBuf,
    archive: PathBuf,
    dataset: PathBuf,
}

impl fmt::Debug for RestoreOperationPaths {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("RestoreOperationPaths(<redacted>)")
    }
}

struct PendingRestore {
    phase: RestorePhase,
    intent_path: PathBuf,
    intent: RestoreIntent,
}

struct RestoreStore {
    app_root: PathBuf,
    prepared: PathBuf,
    old_staged: PathBuf,
    new_installed: PathBuf,
    finalizing: PathBuf,
    staging: PathBuf,
    rollback: PathBuf,
}

impl fmt::Debug for RestoreStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("RestoreStore(<redacted>)")
    }
}

impl RestoreStore {
    fn open_or_create(app_root: &Path) -> Result<Self, RestoreArchiveError> {
        let app_root = require_app_root(app_root).map_err(|_| restore_source_invalid())?;
        let maintenance = ensure_direct_child_directory(&app_root, MAINTENANCE_DIRECTORY)
            .map_err(|_| restore_stage_failed())?;
        let restore_root = ensure_direct_child_directory(&maintenance, RESTORE_DIRECTORY)
            .map_err(|_| restore_stage_failed())?;
        let intents = ensure_direct_child_directory(&restore_root, INTENTS_DIRECTORY)
            .map_err(|_| restore_stage_failed())?;
        let prepared = ensure_direct_child_directory(&intents, PREPARED_DIRECTORY)
            .map_err(|_| restore_stage_failed())?;
        let old_staged = ensure_direct_child_directory(&intents, OLD_STAGED_DIRECTORY)
            .map_err(|_| restore_stage_failed())?;
        let new_installed = ensure_direct_child_directory(&intents, NEW_INSTALLED_DIRECTORY)
            .map_err(|_| restore_stage_failed())?;
        let finalizing = ensure_direct_child_directory(&intents, FINALIZING_DIRECTORY)
            .map_err(|_| restore_stage_failed())?;
        let staging = ensure_direct_child_directory(&restore_root, STAGING_DIRECTORY)
            .map_err(|_| restore_stage_failed())?;
        let rollback = ensure_direct_child_directory(&restore_root, ROLLBACK_DIRECTORY)
            .map_err(|_| restore_stage_failed())?;
        let store = Self {
            app_root,
            prepared,
            old_staged,
            new_installed,
            finalizing,
            staging,
            rollback,
        };
        store.validate_static_control_shape()?;
        Ok(store)
    }

    fn open_existing(app_root: &Path) -> Result<Option<Self>, RestoreArchiveError> {
        let app_root = require_app_root(app_root).map_err(|_| restore_recovery_failed())?;
        let maintenance = app_root.join(MAINTENANCE_DIRECTORY);
        if !path_exists(&maintenance).map_err(|_| restore_recovery_failed())? {
            return Ok(None);
        }
        require_exact_child_directory(&app_root, &maintenance)
            .map_err(|_| restore_recovery_failed())?;
        let restore_root = maintenance.join(RESTORE_DIRECTORY);
        if !path_exists(&restore_root).map_err(|_| restore_recovery_failed())? {
            return Ok(None);
        }
        require_exact_child_directory(&maintenance, &restore_root)
            .map_err(|_| restore_recovery_failed())?;
        let intents = restore_root.join(INTENTS_DIRECTORY);
        let prepared = intents.join(PREPARED_DIRECTORY);
        let old_staged = intents.join(OLD_STAGED_DIRECTORY);
        let new_installed = intents.join(NEW_INSTALLED_DIRECTORY);
        let finalizing = intents.join(FINALIZING_DIRECTORY);
        let staging = restore_root.join(STAGING_DIRECTORY);
        let rollback = restore_root.join(ROLLBACK_DIRECTORY);
        for (parent, child) in [
            (&restore_root, &intents),
            (&intents, &prepared),
            (&intents, &old_staged),
            (&intents, &new_installed),
            (&intents, &finalizing),
            (&restore_root, &staging),
            (&restore_root, &rollback),
        ] {
            require_exact_child_directory(parent, child).map_err(|_| restore_recovery_failed())?;
        }
        let store = Self {
            app_root,
            prepared,
            old_staged,
            new_installed,
            finalizing,
            staging,
            rollback,
        };
        store.validate_static_control_shape()?;
        Ok(Some(store))
    }

    fn validate_static_control_shape(&self) -> Result<(), RestoreArchiveError> {
        let restore_root = self.staging.parent().ok_or_else(restore_intent_conflict)?;
        let intents = self.prepared.parent().ok_or_else(restore_intent_conflict)?;
        require_directory_children(restore_root, &["intents", "rollback", "staging"], true)
            .map_err(|_| restore_intent_conflict())?;
        require_directory_children(
            intents,
            &[
                PREPARED_DIRECTORY,
                OLD_STAGED_DIRECTORY,
                NEW_INSTALLED_DIRECTORY,
                FINALIZING_DIRECTORY,
            ],
            true,
        )
        .map_err(|_| restore_intent_conflict())?;
        Ok(())
    }

    fn require_no_pending_or_conflicting_intent(&self) -> Result<(), RestoreArchiveError> {
        if self.list_intents()?.is_empty() && !conflicting_clear_intent(&self.app_root)? {
            self.require_empty_operation_roots()?;
            Ok(())
        } else {
            Err(restore_intent_conflict())
        }
    }

    fn require_empty_operation_roots(&self) -> Result<(), RestoreArchiveError> {
        for directory in [&self.staging, &self.rollback] {
            if fs::read_dir(directory)
                .map_err(|_| restore_intent_conflict())?
                .next()
                .is_some()
            {
                return Err(restore_intent_conflict());
            }
        }
        Ok(())
    }

    fn create_stage(
        &self,
        operation_id: Uuid,
    ) -> Result<RestoreOperationPaths, RestoreArchiveError> {
        let root = ensure_direct_child_directory(&self.staging, &operation_id.to_string())
            .map_err(|_| restore_stage_failed())?;
        let dataset = ensure_direct_child_directory(&root, STAGED_DATASET)
            .map_err(|_| restore_stage_failed())?;
        Ok(RestoreOperationPaths {
            archive: root.join(STAGED_ARCHIVE),
            root,
            dataset,
        })
    }

    fn operation(&self, operation_id: Uuid) -> RestoreOperationPaths {
        let root = self.staging.join(operation_id.to_string());
        RestoreOperationPaths {
            archive: root.join(STAGED_ARCHIVE),
            dataset: root.join(STAGED_DATASET),
            root,
        }
    }

    fn rollback_root(&self, operation_id: Uuid) -> PathBuf {
        self.rollback.join(operation_id.to_string())
    }

    fn write_prepared_intent(&self, intent: &RestoreIntent) -> Result<(), RestoreArchiveError> {
        let target = self.intent_path(RestorePhase::Prepared, intent.operation_id);
        if path_exists(&target).map_err(|_| restore_stage_failed())? {
            return Err(restore_intent_conflict());
        }
        let temporary = self
            .prepared
            .join(format!(".restore-{}.tmp", intent.operation_id));
        let bytes = serde_json::to_vec(intent).map_err(|_| restore_stage_failed())?;
        if bytes.is_empty() || bytes.len() > MAX_INTENT_BYTES as usize {
            return Err(restore_stage_failed());
        }
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|_| restore_stage_failed())?;
        let write = file
            .write_all(&bytes)
            .and_then(|()| file.sync_all())
            .map_err(|_| restore_stage_failed());
        drop(file);
        if let Err(error) = write {
            let _ = fs::remove_file(&temporary);
            return Err(error);
        }
        rename_checked(&self.app_root, &temporary, &target, OwnedEntryKind::File)
            .map_err(|_| restore_stage_failed())?;
        sync(&self.prepared).map_err(|_| restore_stage_failed())
    }

    fn transition(
        &self,
        pending: &mut PendingRestore,
        next: RestorePhase,
    ) -> Result<(), RestoreArchiveError> {
        let target = self.intent_path(next, pending.intent.operation_id);
        rename_checked(
            &self.app_root,
            &pending.intent_path,
            &target,
            OwnedEntryKind::File,
        )
        .map_err(|_| restore_recovery_failed())?;
        pending.intent_path = target;
        pending.phase = next;
        Ok(())
    }

    fn intent_path(&self, phase: RestorePhase, operation_id: Uuid) -> PathBuf {
        let parent = match phase {
            RestorePhase::Prepared => &self.prepared,
            RestorePhase::OldStaged => &self.old_staged,
            RestorePhase::NewInstalled => &self.new_installed,
            RestorePhase::Finalizing => &self.finalizing,
        };
        parent.join(format!("{operation_id}.json"))
    }

    fn list_intents(&self) -> Result<Vec<PendingRestore>, RestoreArchiveError> {
        let mut pending = Vec::new();
        for (phase, directory) in [
            (RestorePhase::Prepared, &self.prepared),
            (RestorePhase::OldStaged, &self.old_staged),
            (RestorePhase::NewInstalled, &self.new_installed),
            (RestorePhase::Finalizing, &self.finalizing),
        ] {
            for entry in fs::read_dir(directory).map_err(|_| restore_intent_conflict())? {
                if pending.len() >= MAX_CONTROL_ENTRIES {
                    return Err(restore_intent_conflict());
                }
                let entry = entry.map_err(|_| restore_intent_conflict())?;
                let path = entry.path();
                if path.parent() != Some(directory.as_path()) {
                    return Err(restore_intent_conflict());
                }
                let file_name = entry.file_name();
                let name = file_name.to_str().ok_or_else(restore_intent_conflict)?;
                if name.starts_with(".restore-") && name.ends_with(".tmp") {
                    return Err(restore_intent_conflict());
                }
                let operation_id = Path::new(name)
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .and_then(parse_canonical_uuid)
                    .filter(|_| {
                        Path::new(name).extension().and_then(|ext| ext.to_str()) == Some("json")
                    })
                    .ok_or_else(restore_intent_conflict)?;
                let intent = read_intent(&path)?;
                intent.validate(operation_id)?;
                pending.push(PendingRestore {
                    phase,
                    intent_path: path,
                    intent,
                });
            }
        }
        if pending.len() > 1 {
            return Err(restore_intent_conflict());
        }
        Ok(pending)
    }

    fn has_intent(&self, operation_id: Uuid) -> Result<bool, RestoreArchiveError> {
        Ok(self
            .list_intents()?
            .iter()
            .any(|pending| pending.intent.operation_id == operation_id))
    }

    fn remove_uncommitted_stage(
        &self,
        operation: &RestoreOperationPaths,
    ) -> Result<(), RestoreArchiveError> {
        if path_exists(&operation.root).map_err(|_| restore_stage_failed())? {
            remove_tree(&self.app_root, &operation.root, OwnedEntryKind::Directory)
                .map_err(|_| restore_stage_failed())?;
        }
        Ok(())
    }
}

pub fn recover_pending_restore(app_root: &Path) -> AppResult<bool> {
    recover_pending_restore_with_fault(app_root, &NoRestoreFault)
        .map_err(|_| AppError::new(AppErrorCode::RequestConflict))
}

#[doc(hidden)]
pub fn recover_pending_restore_with_fault(
    app_root: &Path,
    fault: &dyn RestoreFaultInjector,
) -> Result<bool, RestoreArchiveError> {
    let Some(store) = RestoreStore::open_existing(app_root)? else {
        return Ok(false);
    };
    if conflicting_clear_intent(&store.app_root)? {
        return Err(restore_intent_conflict());
    }
    let mut intents = store.list_intents()?;
    let Some(mut pending) = intents.pop() else {
        store.require_empty_operation_roots()?;
        remove_empty_restore_control(&store)?;
        return Ok(false);
    };
    let operation_id = pending.intent.operation_id;
    let operation = store.operation(operation_id);
    let rollback = store.rollback_root(operation_id);
    validate_operation_paths(&store, &operation, &rollback, pending.phase)?;
    let verified = if pending.phase == RestorePhase::Finalizing {
        None
    } else {
        let verified =
            verify_archive_file(&operation.archive).map_err(|_| restore_archive_invalid())?;
        validate_verified_intent(&pending.intent, &verified)?;
        Some(verified)
    };

    if pending.phase == RestorePhase::Prepared {
        let verified = verified.as_ref().ok_or_else(restore_recovery_failed)?;
        validate_materialized_dataset(&operation.dataset, verified)?;
        stage_old_state(&store, &operation, &rollback, fault)?;
        store.transition(&mut pending, RestorePhase::OldStaged)?;
        checkpoint(fault, RestoreBoundary::OldStateStaged)?;
    }
    if pending.phase == RestorePhase::OldStaged {
        let verified = verified.as_ref().ok_or_else(restore_recovery_failed)?;
        validate_split_new_state(&store.app_root, &operation.dataset, verified)?;
        install_new_state(&store, &operation, &rollback, fault)?;
        validate_installed_dataset(&store.app_root, verified)?;
        store.transition(&mut pending, RestorePhase::NewInstalled)?;
        checkpoint(fault, RestoreBoundary::NewStateInstalled)?;
    }
    if pending.phase == RestorePhase::NewInstalled {
        let verified = verified.as_ref().ok_or_else(restore_recovery_failed)?;
        validate_installed_dataset(&store.app_root, verified)?;
        checkpoint(fault, RestoreBoundary::BeforeRollbackCleanup)?;
        store.transition(&mut pending, RestorePhase::Finalizing)?;
        checkpoint(fault, RestoreBoundary::FinalizingIntentWritten)?;
    }
    if pending.phase == RestorePhase::Finalizing {
        if path_exists(&rollback).map_err(|_| restore_recovery_failed())? {
            let snapshot = capture_tree(&store.app_root, &rollback, OwnedEntryKind::Directory)
                .map_err(|_| restore_recovery_failed())?;
            remove_captured_tree(&store.app_root, &snapshot)
                .map_err(|_| restore_recovery_failed())?;
        }
        checkpoint(fault, RestoreBoundary::BeforeStageCleanup)?;
        if path_exists(&operation.root).map_err(|_| restore_recovery_failed())? {
            let snapshot =
                capture_tree(&store.app_root, &operation.root, OwnedEntryKind::Directory)
                    .map_err(|_| restore_recovery_failed())?;
            remove_captured_tree(&store.app_root, &snapshot)
                .map_err(|_| restore_recovery_failed())?;
        }
        checkpoint(fault, RestoreBoundary::BeforeIntentCleanup)?;
        require_regular_file(&pending.intent_path).map_err(|_| restore_recovery_failed())?;
        fs::remove_file(&pending.intent_path).map_err(|_| restore_recovery_failed())?;
        sync(
            pending
                .intent_path
                .parent()
                .ok_or_else(restore_recovery_failed)?,
        )
        .map_err(|_| restore_recovery_failed())?;
        remove_empty_restore_control(&store)?;
    }
    Ok(true)
}

fn remove_empty_restore_control(store: &RestoreStore) -> Result<(), RestoreArchiveError> {
    let restore_root = store.staging.parent().ok_or_else(restore_recovery_failed)?;
    if path_exists(restore_root).map_err(|_| restore_recovery_failed())? {
        let snapshot = capture_tree(&store.app_root, restore_root, OwnedEntryKind::Directory)
            .map_err(|_| restore_recovery_failed())?;
        if !snapshot.is_directory_only() {
            return Err(restore_recovery_failed());
        }
        remove_captured_tree(&store.app_root, &snapshot).map_err(|_| restore_recovery_failed())?;
    }
    remove_empty_maintenance_directory(&store.app_root)
}

fn remove_empty_maintenance_directory(app_root: &Path) -> Result<(), RestoreArchiveError> {
    let maintenance = app_root.join(MAINTENANCE_DIRECTORY);
    if path_exists(&maintenance).map_err(|_| restore_recovery_failed())?
        && fs::read_dir(&maintenance)
            .map_err(|_| restore_recovery_failed())?
            .next()
            .is_none()
    {
        fs::remove_dir(&maintenance).map_err(|_| restore_recovery_failed())?;
        sync(app_root).map_err(|_| restore_recovery_failed())?;
    }
    Ok(())
}

fn validate_operation_paths(
    store: &RestoreStore,
    operation: &RestoreOperationPaths,
    rollback: &Path,
    phase: RestorePhase,
) -> Result<(), RestoreArchiveError> {
    let operation_name = operation_id_text(operation)?;
    let staging_children =
        require_directory_children(&store.staging, &[operation_name.as_str()], false)
            .map_err(|_| restore_recovery_failed())?;
    let rollback_children =
        require_directory_children(&store.rollback, &[operation_name.as_str()], false)
            .map_err(|_| restore_recovery_failed())?;
    if phase == RestorePhase::Finalizing {
        if rollback_children.len() == 1 {
            require_exact_child_directory(&store.rollback, rollback)
                .map_err(|_| restore_recovery_failed())?;
            validate_rollback_shape(store, rollback)?;
        }
        if staging_children.len() == 1 {
            require_exact_child_directory(&store.staging, &operation.root)
                .map_err(|_| restore_recovery_failed())?;
        }
        return Ok(());
    }
    if staging_children.len() != 1 {
        return Err(restore_recovery_failed());
    }
    require_exact_child_directory(&store.staging, &operation.root)
        .map_err(|_| restore_recovery_failed())?;
    validate_operation_root_shape(operation)?;
    require_regular_file(&operation.archive).map_err(|_| restore_recovery_failed())?;
    require_exact_child_directory(&operation.root, &operation.dataset)
        .map_err(|_| restore_recovery_failed())?;
    if phase == RestorePhase::Prepared {
        if rollback_children.len() == 1 {
            require_exact_child_directory(&store.rollback, rollback)
                .map_err(|_| restore_recovery_failed())?;
        }
    } else {
        if rollback_children.len() != 1 {
            return Err(restore_recovery_failed());
        }
        require_exact_child_directory(&store.rollback, rollback)
            .map_err(|_| restore_recovery_failed())?;
    }
    if !rollback_children.is_empty() {
        validate_rollback_shape(store, rollback)?;
    }
    Ok(())
}

fn validate_operation_root_shape(
    operation: &RestoreOperationPaths,
) -> Result<(), RestoreArchiveError> {
    let mut seen = HashSet::new();
    for entry in fs::read_dir(&operation.root).map_err(|_| restore_recovery_failed())? {
        let entry = entry.map_err(|_| restore_recovery_failed())?;
        let name = entry
            .file_name()
            .to_str()
            .ok_or_else(restore_recovery_failed)?
            .to_owned();
        if entry.path().parent() != Some(operation.root.as_path()) || !seen.insert(name.clone()) {
            return Err(restore_recovery_failed());
        }
        match name.as_str() {
            STAGED_ARCHIVE => {
                require_regular_file(&entry.path()).map_err(|_| restore_recovery_failed())?
            }
            STAGED_DATASET => require_exact_child_directory(&operation.root, &entry.path())
                .map_err(|_| restore_recovery_failed())?,
            _ => return Err(restore_recovery_failed()),
        }
    }
    if seen != HashSet::from([STAGED_ARCHIVE.to_owned(), STAGED_DATASET.to_owned()]) {
        return Err(restore_recovery_failed());
    }
    Ok(())
}

fn validate_rollback_shape(
    store: &RestoreStore,
    rollback: &Path,
) -> Result<(), RestoreArchiveError> {
    let allowed = data_units_with_sidecars()
        .into_iter()
        .map(|(name, kind, _)| (name, kind))
        .collect::<HashMap<_, _>>();
    let mut seen = HashSet::new();
    for entry in fs::read_dir(rollback).map_err(|_| restore_recovery_failed())? {
        let entry = entry.map_err(|_| restore_recovery_failed())?;
        let name = entry
            .file_name()
            .to_str()
            .ok_or_else(restore_recovery_failed)?
            .to_owned();
        let kind = allowed
            .get(name.as_str())
            .copied()
            .ok_or_else(restore_recovery_failed)?;
        if entry.path().parent() != Some(rollback) || !seen.insert(name) {
            return Err(restore_recovery_failed());
        }
        match kind {
            OwnedEntryKind::File => {
                require_regular_file(&entry.path()).map_err(|_| restore_recovery_failed())?
            }
            OwnedEntryKind::Directory => require_exact_child_directory(rollback, &entry.path())
                .map_err(|_| restore_recovery_failed())?,
        }
        capture_tree(&store.app_root, &entry.path(), kind)
            .map_err(|_| restore_recovery_failed())?;
    }
    Ok(())
}

fn stage_old_state(
    store: &RestoreStore,
    operation: &RestoreOperationPaths,
    rollback: &Path,
    fault: &dyn RestoreFaultInjector,
) -> Result<(), RestoreArchiveError> {
    if !path_exists(rollback).map_err(|_| restore_recovery_failed())? {
        ensure_direct_child_directory(&store.rollback, &operation_id_text(operation)?)
            .map_err(|_| restore_recovery_failed())?;
    }
    require_exact_child_directory(&store.rollback, rollback)
        .map_err(|_| restore_recovery_failed())?;
    validate_root_entries(&store.app_root)?;
    let units = data_units_with_sidecars();
    for (index, (name, kind, required)) in units.iter().enumerate() {
        let current = store.app_root.join(name);
        let old = rollback.join(name);
        let current_exists = path_exists(&current).map_err(|_| restore_recovery_failed())?;
        let old_exists = path_exists(&old).map_err(|_| restore_recovery_failed())?;
        match (current_exists, old_exists, required) {
            (true, false, _) => {
                checkpoint(fault, RestoreBoundary::BeforeOldMove(index))?;
                capture_tree(&store.app_root, &current, *kind)
                    .map_err(|_| restore_recovery_failed())?;
                rename_checked(&store.app_root, &current, &old, *kind)
                    .map_err(|_| restore_recovery_failed())?;
                checkpoint(fault, RestoreBoundary::AfterOldMove(index))?;
            }
            (false, true, _) => {}
            (false, false, false) => {}
            _ => return Err(restore_recovery_failed()),
        }
    }
    for (name, _, required) in &units {
        if path_exists(&store.app_root.join(name)).map_err(|_| restore_recovery_failed())?
            || (*required
                && !path_exists(&rollback.join(name)).map_err(|_| restore_recovery_failed())?)
        {
            return Err(restore_recovery_failed());
        }
    }
    sync(&store.app_root).map_err(|_| restore_recovery_failed())?;
    sync(rollback).map_err(|_| restore_recovery_failed())
}

fn install_new_state(
    store: &RestoreStore,
    operation: &RestoreOperationPaths,
    rollback: &Path,
    fault: &dyn RestoreFaultInjector,
) -> Result<(), RestoreArchiveError> {
    require_exact_child_directory(&store.rollback, rollback)
        .map_err(|_| restore_recovery_failed())?;
    for (index, (name, kind)) in data_units().iter().enumerate() {
        let staged = operation.dataset.join(name);
        let current = store.app_root.join(name);
        let staged_exists = path_exists(&staged).map_err(|_| restore_recovery_failed())?;
        let current_exists = path_exists(&current).map_err(|_| restore_recovery_failed())?;
        match (staged_exists, current_exists) {
            (true, false) => {
                checkpoint(fault, RestoreBoundary::BeforeNewMove(index))?;
                rename_checked(&store.app_root, &staged, &current, *kind)
                    .map_err(|_| restore_recovery_failed())?;
                checkpoint(fault, RestoreBoundary::AfterNewMove(index))?;
            }
            (false, true) => {}
            _ => return Err(restore_recovery_failed()),
        }
    }
    for sidecar in SQLITE_SIDECARS {
        if path_exists(&store.app_root.join(sidecar)).map_err(|_| restore_recovery_failed())? {
            return Err(restore_recovery_failed());
        }
    }
    sync(&store.app_root).map_err(|_| restore_recovery_failed())
}

fn data_units() -> [(&'static str, OwnedEntryKind); 4] {
    [
        ("library.sqlite3", OwnedEntryKind::File),
        ("books", OwnedEntryKind::Directory),
        ("cache", OwnedEntryKind::Directory),
        ("logs", OwnedEntryKind::Directory),
    ]
}

fn data_units_with_sidecars() -> Vec<(&'static str, OwnedEntryKind, bool)> {
    let mut units = data_units()
        .into_iter()
        .map(|(name, kind)| (name, kind, true))
        .collect::<Vec<_>>();
    units.extend(
        SQLITE_SIDECARS
            .into_iter()
            .map(|name| (name, OwnedEntryKind::File, false)),
    );
    units
}

fn operation_id_text(operation: &RestoreOperationPaths) -> Result<String, RestoreArchiveError> {
    operation
        .root
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(parse_canonical_uuid)
        .map(|id| id.to_string())
        .ok_or_else(restore_recovery_failed)
}

fn prepare_empty_runtime_directories(dataset: &Path) -> Result<(), RestoreArchiveError> {
    for name in ["books", "cache", "logs"] {
        let path = dataset.join(name);
        if safe_path_exists(&path).map_err(|_| restore_stage_failed())? {
            require_safe_directory(&path).map_err(|_| restore_stage_failed())?;
        } else {
            fs::create_dir(&path).map_err(|_| restore_stage_failed())?;
        }
    }
    sync_app_directory(dataset).map_err(|_| restore_stage_failed())
}

fn validate_materialized_dataset(
    dataset: &Path,
    verified: &VerifiedBackup,
) -> Result<(), RestoreArchiveError> {
    let mut expected_files = HashMap::new();
    let mut expected_directories = HashSet::from([
        String::new(),
        "books".to_owned(),
        "cache".to_owned(),
        "logs".to_owned(),
    ]);
    for entry in &verified.entries {
        if expected_files.insert(entry.path.clone(), entry).is_some() {
            return Err(restore_archive_invalid());
        }
        let mut parts = entry.path.split('/').collect::<Vec<_>>();
        parts.pop();
        while !parts.is_empty() {
            expected_directories.insert(parts.join("/"));
            parts.pop();
        }
        verify_materialized_file(
            &dataset.join(entry.path.replace('/', std::path::MAIN_SEPARATOR_STR)),
            entry.size,
            &entry.sha256,
        )
        .map_err(|_| restore_archive_invalid())?;
    }
    let mut actual_files = HashSet::new();
    let mut actual_directories = HashSet::new();
    walk_materialized(
        dataset,
        dataset,
        &mut actual_files,
        &mut actual_directories,
        0,
    )?;
    if actual_files != expected_files.keys().cloned().collect()
        || actual_directories != expected_directories
    {
        return Err(restore_archive_invalid());
    }
    Ok(())
}

fn walk_materialized(
    root: &Path,
    directory: &Path,
    files: &mut HashSet<String>,
    directories: &mut HashSet<String>,
    depth: usize,
) -> Result<(), RestoreArchiveError> {
    if depth > 32 || files.len().saturating_add(directories.len()) > 100_000 {
        return Err(restore_archive_invalid());
    }
    let metadata = fs::symlink_metadata(directory).map_err(|_| restore_archive_invalid())?;
    if !metadata.is_dir() || metadata_is_reparse(&metadata) {
        return Err(restore_archive_invalid());
    }
    let relative = directory
        .strip_prefix(root)
        .map_err(|_| restore_archive_invalid())?;
    let relative_text = relative_path_text(relative)?;
    directories.insert(relative_text);
    let mut children = fs::read_dir(directory)
        .map_err(|_| restore_archive_invalid())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| restore_archive_invalid())?;
    children.sort_by_key(|entry| entry.file_name().to_string_lossy().to_ascii_lowercase());
    for child in children {
        let path = child.path();
        if path.parent() != Some(directory) {
            return Err(restore_archive_invalid());
        }
        let metadata = fs::symlink_metadata(&path).map_err(|_| restore_archive_invalid())?;
        if metadata_is_reparse(&metadata) {
            return Err(restore_archive_invalid());
        }
        if metadata.is_dir() {
            walk_materialized(root, &path, files, directories, depth + 1)?;
        } else if metadata.is_file() {
            require_regular_file(&path).map_err(|_| restore_archive_invalid())?;
            let relative = path
                .strip_prefix(root)
                .map_err(|_| restore_archive_invalid())?;
            if !files.insert(relative_path_text(relative)?) {
                return Err(restore_archive_invalid());
            }
        } else {
            return Err(restore_archive_invalid());
        }
    }
    Ok(())
}

fn validate_split_new_state(
    app_root: &Path,
    staged_dataset: &Path,
    verified: &VerifiedBackup,
) -> Result<(), RestoreArchiveError> {
    let virtual_root = tempfile_virtual_root(app_root, staged_dataset)?;
    for entry in &verified.entries {
        let first = entry
            .path
            .split('/')
            .next()
            .ok_or_else(restore_archive_invalid)?;
        let base = virtual_root
            .get(first)
            .ok_or_else(restore_archive_invalid)?;
        let suffix = entry
            .path
            .strip_prefix(first)
            .ok_or_else(restore_archive_invalid)?;
        let suffix = suffix.strip_prefix('/').unwrap_or(suffix);
        let path = if suffix.is_empty() {
            base.clone()
        } else {
            base.join(suffix.replace('/', std::path::MAIN_SEPARATOR_STR))
        };
        verify_materialized_file(&path, entry.size, &entry.sha256)
            .map_err(|_| restore_archive_invalid())?;
    }
    validate_virtual_units_exact(app_root, staged_dataset, verified)
}

fn tempfile_virtual_root(
    app_root: &Path,
    staged_dataset: &Path,
) -> Result<HashMap<&'static str, PathBuf>, RestoreArchiveError> {
    let mut roots = HashMap::new();
    for (name, _) in data_units() {
        let staged = staged_dataset.join(name);
        let installed = app_root.join(name);
        match (
            path_exists(&staged).map_err(|_| restore_recovery_failed())?,
            path_exists(&installed).map_err(|_| restore_recovery_failed())?,
        ) {
            (true, false) => {
                roots.insert(name, staged);
            }
            (false, true) => {
                roots.insert(name, installed);
            }
            _ => return Err(restore_recovery_failed()),
        }
    }
    Ok(roots)
}

fn validate_virtual_units_exact(
    app_root: &Path,
    staged_dataset: &Path,
    verified: &VerifiedBackup,
) -> Result<(), RestoreArchiveError> {
    let roots = tempfile_virtual_root(app_root, staged_dataset)?;
    let mut expected_by_unit: HashMap<&str, HashSet<String>> = HashMap::new();
    let mut expected_directories_by_unit: HashMap<&str, HashSet<String>> = HashMap::new();
    for entry in &verified.entries {
        let (unit, suffix) = entry.path.split_once('/').unwrap_or((&entry.path, ""));
        expected_by_unit
            .entry(unit)
            .or_default()
            .insert(suffix.to_owned());
        let directories = expected_directories_by_unit
            .entry(unit)
            .or_insert_with(|| HashSet::from([String::new()]));
        if !suffix.is_empty() {
            let mut parts = suffix.split('/').collect::<Vec<_>>();
            parts.pop();
            while !parts.is_empty() {
                directories.insert(parts.join("/"));
                parts.pop();
            }
        }
    }
    for unit in ["books", "cache", "logs"] {
        expected_directories_by_unit
            .entry(unit)
            .or_insert_with(|| HashSet::from([String::new()]));
    }
    for (unit, root) in roots {
        if unit == "library.sqlite3" {
            require_regular_file(&root).map_err(|_| restore_recovery_failed())?;
            continue;
        }
        let mut files = HashSet::new();
        let mut directories = HashSet::new();
        walk_materialized(&root, &root, &mut files, &mut directories, 0)?;
        let expected = expected_by_unit.remove(unit).unwrap_or_default();
        let expected_directories = expected_directories_by_unit
            .remove(unit)
            .unwrap_or_else(|| HashSet::from([String::new()]));
        if files != expected || directories != expected_directories {
            return Err(restore_archive_invalid());
        }
        if matches!(unit, "cache" | "logs") && !files.is_empty() {
            return Err(restore_archive_invalid());
        }
    }
    Ok(())
}

fn validate_installed_dataset(
    app_root: &Path,
    verified: &VerifiedBackup,
) -> Result<(), RestoreArchiveError> {
    for entry in &verified.entries {
        let path = app_root.join(entry.path.replace('/', std::path::MAIN_SEPARATOR_STR));
        verify_materialized_file(&path, entry.size, &entry.sha256)
            .map_err(|_| restore_archive_invalid())?;
    }
    for sidecar in SQLITE_SIDECARS {
        if path_exists(&app_root.join(sidecar)).map_err(|_| restore_recovery_failed())? {
            return Err(restore_recovery_failed());
        }
    }
    validate_virtual_units_exact(app_root, &app_root.join(".no-staged-dataset"), verified)
}

async fn preflight_database(
    operation: &RestoreOperationPaths,
    _verified: &VerifiedBackup,
) -> Result<Vec<Uuid>, RestoreArchiveError> {
    let database_path = operation.dataset.join("library.sqlite3");
    let options = SqliteConnectOptions::new()
        .filename(&database_path)
        .create_if_missing(false)
        .read_only(true)
        .immutable(true)
        .foreign_keys(true);
    let mut connection = SqliteConnection::connect_with(&options)
        .await
        .map_err(|_| restore_preflight_failed())?;
    sqlx::query("PRAGMA query_only = ON")
        .execute(&mut connection)
        .await
        .map_err(|_| restore_preflight_failed())?;
    require_database_integrity(&mut connection).await?;
    validate_migrations(&mut connection).await?;
    validate_schema(&mut connection, operation).await?;
    let forbidden_remote_rows: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM provider_remote_resources")
            .fetch_one(&mut connection)
            .await
            .map_err(|_| restore_preflight_failed())?;
    if forbidden_remote_rows != 0 {
        return Err(restore_preflight_failed());
    }
    let profile_rows = sqlx::query("SELECT id FROM provider_profiles ORDER BY id LIMIT ?")
        .bind(i64::try_from(MAX_BOOK_ROWS + 1).map_err(|_| restore_preflight_failed())?)
        .fetch_all(&mut connection)
        .await
        .map_err(|_| restore_preflight_failed())?;
    if profile_rows.len() > MAX_BOOK_ROWS {
        return Err(restore_preflight_failed());
    }
    let mut profiles = Vec::with_capacity(profile_rows.len());
    let mut seen = HashSet::new();
    for row in profile_rows {
        let value: String = row.try_get("id").map_err(|_| restore_preflight_failed())?;
        let id = parse_canonical_uuid(&value).ok_or_else(restore_preflight_failed)?;
        if !seen.insert(id) {
            return Err(restore_preflight_failed());
        }
        profiles.push(id);
    }
    connection
        .close()
        .await
        .map_err(|_| restore_preflight_failed())?;
    Ok(profiles)
}

async fn require_database_integrity(
    connection: &mut SqliteConnection,
) -> Result<(), RestoreArchiveError> {
    let integrity = sqlx::query_scalar::<_, String>("PRAGMA integrity_check")
        .fetch_all(&mut *connection)
        .await
        .map_err(|_| restore_preflight_failed())?;
    if integrity.as_slice() != ["ok"] {
        return Err(restore_preflight_failed());
    }
    if sqlx::query("PRAGMA foreign_key_check")
        .fetch_optional(&mut *connection)
        .await
        .map_err(|_| restore_preflight_failed())?
        .is_some()
    {
        return Err(restore_preflight_failed());
    }
    Ok(())
}

async fn validate_migrations(connection: &mut SqliteConnection) -> Result<(), RestoreArchiveError> {
    let rows =
        sqlx::query("SELECT version, checksum, success FROM _sqlx_migrations ORDER BY version")
            .fetch_all(&mut *connection)
            .await
            .map_err(|_| restore_preflight_failed())?;
    if rows.len() != MIGRATOR.migrations.len() {
        return Err(restore_preflight_failed());
    }
    for (row, migration) in rows.iter().zip(MIGRATOR.migrations.iter()) {
        let version: i64 = row
            .try_get("version")
            .map_err(|_| restore_preflight_failed())?;
        let checksum: Vec<u8> = row
            .try_get("checksum")
            .map_err(|_| restore_preflight_failed())?;
        let success: bool = row
            .try_get("success")
            .map_err(|_| restore_preflight_failed())?;
        if version != migration.version
            || checksum.as_slice() != migration.checksum.as_ref()
            || !success
        {
            return Err(restore_preflight_failed());
        }
    }
    Ok(())
}

async fn validate_schema(
    connection: &mut SqliteConnection,
    operation: &RestoreOperationPaths,
) -> Result<(), RestoreArchiveError> {
    let actual = schema_rows(connection).await?;
    let reference_path = operation.root.join("reference.sqlite3");
    if path_exists(&reference_path).map_err(|_| restore_preflight_failed())? {
        return Err(restore_preflight_failed());
    }
    let options = SqliteConnectOptions::new()
        .filename(&reference_path)
        .create_if_missing(true)
        .foreign_keys(true);
    let mut reference = SqliteConnection::connect_with(&options)
        .await
        .map_err(|_| restore_preflight_failed())?;
    MIGRATOR
        .run(&mut reference)
        .await
        .map_err(|_| restore_preflight_failed())?;
    let expected = schema_rows(&mut reference).await?;
    reference
        .close()
        .await
        .map_err(|_| restore_preflight_failed())?;
    if actual != expected {
        cleanup_reference_files(&operation.root)?;
        return Err(restore_preflight_failed());
    }
    cleanup_reference_files(&operation.root)
}

async fn schema_rows(
    connection: &mut SqliteConnection,
) -> Result<Vec<(String, String, String, Option<String>)>, RestoreArchiveError> {
    let rows = sqlx::query(
        "SELECT type, name, tbl_name, sql FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%' ORDER BY type, name, tbl_name",
    )
    .fetch_all(&mut *connection)
    .await
    .map_err(|_| restore_preflight_failed())?;
    rows.into_iter()
        .map(|row| {
            let sql: Option<String> = row.try_get("sql").map_err(|_| restore_preflight_failed())?;
            Ok((
                row.try_get("type")
                    .map_err(|_| restore_preflight_failed())?,
                row.try_get("name")
                    .map_err(|_| restore_preflight_failed())?,
                row.try_get("tbl_name")
                    .map_err(|_| restore_preflight_failed())?,
                sql.map(|value| value.replace("\r\n", "\n").replace('\r', "\n")),
            ))
        })
        .collect()
}

fn cleanup_reference_files(operation_root: &Path) -> Result<(), RestoreArchiveError> {
    for name in [
        "reference.sqlite3",
        "reference.sqlite3-wal",
        "reference.sqlite3-shm",
        "reference.sqlite3-journal",
    ] {
        let path = operation_root.join(name);
        if path_exists(&path).map_err(|_| restore_preflight_failed())? {
            require_regular_file(&path).map_err(|_| restore_preflight_failed())?;
            fs::remove_file(&path).map_err(|_| restore_preflight_failed())?;
        }
    }
    sync(operation_root).map_err(|_| restore_preflight_failed())
}

async fn validate_archive_ownership(
    dataset: &Path,
    verified: &VerifiedBackup,
) -> Result<(), RestoreArchiveError> {
    let options = SqliteConnectOptions::new()
        .filename(dataset.join("library.sqlite3"))
        .create_if_missing(false)
        .read_only(true)
        .immutable(true)
        .foreign_keys(true);
    let mut connection = SqliteConnection::connect_with(&options)
        .await
        .map_err(|_| restore_preflight_failed())?;
    let limit = i64::try_from(MAX_BOOK_ROWS + 1).map_err(|_| restore_preflight_failed())?;
    let rows = sqlx::query(
        "SELECT id, format, stored_path, sha256, import_status FROM books ORDER BY id LIMIT ?",
    )
    .bind(limit)
    .fetch_all(&mut connection)
    .await
    .map_err(|_| restore_preflight_failed())?;
    if rows.len() > MAX_BOOK_ROWS {
        return Err(restore_preflight_failed());
    }
    let archive_paths = verified
        .entries
        .iter()
        .map(|entry| entry.path.as_str())
        .collect::<HashSet<_>>();
    let mut books = HashSet::with_capacity(rows.len());
    for row in rows {
        let id_text: String = row.try_get("id").map_err(|_| restore_preflight_failed())?;
        let id = parse_canonical_uuid(&id_text).ok_or_else(restore_preflight_failed)?;
        if !books.insert(id) {
            return Err(restore_preflight_failed());
        }
        let format: String = row
            .try_get("format")
            .map_err(|_| restore_preflight_failed())?;
        let stored_path: Option<String> = row
            .try_get("stored_path")
            .map_err(|_| restore_preflight_failed())?;
        let sha256: Option<String> = row
            .try_get("sha256")
            .map_err(|_| restore_preflight_failed())?;
        let status: String = row
            .try_get("import_status")
            .map_err(|_| restore_preflight_failed())?;
        let expected_source = format!("books/{id}/original.{format}");
        match (&stored_path, &sha256) {
            (Some(path), Some(hash))
                if path == &expected_source
                    && hash.len() == 64
                    && hash
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
                    && archive_paths.contains(path.as_str()) =>
            {
                let entry = verified
                    .entries
                    .iter()
                    .find(|entry| entry.path == *path)
                    .ok_or_else(restore_preflight_failed)?;
                if &entry.sha256 != hash {
                    return Err(restore_preflight_failed());
                }
            }
            (None, None) => {
                let book_prefix = format!("books/{id}/");
                if archive_paths
                    .iter()
                    .any(|path| path.starts_with(book_prefix.as_str()))
                {
                    return Err(restore_preflight_failed());
                }
            }
            _ => return Err(restore_preflight_failed()),
        }
        let derived = format!("books/{id}/derived/document.html");
        let section_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM sections WHERE book_id = ?")
                .bind(id.to_string())
                .fetch_one(&mut connection)
                .await
                .map_err(|_| restore_preflight_failed())?;
        if (status == "ready" || section_count > 0) && !archive_paths.contains(derived.as_str()) {
            return Err(restore_preflight_failed());
        }
    }
    for entry in &verified.entries {
        if entry.path == "library.sqlite3" {
            continue;
        }
        let owner = entry
            .path
            .split('/')
            .nth(1)
            .and_then(parse_canonical_uuid)
            .ok_or_else(restore_preflight_failed)?;
        if !books.contains(&owner) {
            return Err(restore_preflight_failed());
        }
    }
    connection
        .close()
        .await
        .map_err(|_| restore_preflight_failed())?;
    Ok(())
}

fn validate_verified_intent(
    intent: &RestoreIntent,
    verified: &VerifiedBackup,
) -> Result<(), RestoreArchiveError> {
    if intent.archive_bytes != verified.archive_bytes
        || intent.entry_count != verified.entry_count
        || intent.payload_bytes != verified.total_bytes
        || intent.manifest_sha256 != verified.manifest_sha256
    {
        return Err(restore_intent_conflict());
    }
    Ok(())
}

fn validate_root_entries(root: &Path) -> Result<(), RestoreArchiveError> {
    let allowed = HashSet::from([
        "library.sqlite3",
        "library.sqlite3-wal",
        "library.sqlite3-shm",
        "library.sqlite3-journal",
        "books",
        "cache",
        "logs",
        MAINTENANCE_DIRECTORY,
    ]);
    let mut seen = HashSet::new();
    for entry in fs::read_dir(root).map_err(|_| restore_recovery_failed())? {
        let entry = entry.map_err(|_| restore_recovery_failed())?;
        let file_name = entry.file_name();
        let name = file_name.to_str().ok_or_else(restore_recovery_failed)?;
        let normalized = name.to_ascii_lowercase();
        if name != normalized || !allowed.contains(name) || !seen.insert(normalized) {
            return Err(restore_recovery_failed());
        }
        let metadata = fs::symlink_metadata(entry.path()).map_err(|_| restore_recovery_failed())?;
        if metadata_is_reparse(&metadata) || (!metadata.is_file() && !metadata.is_dir()) {
            return Err(restore_recovery_failed());
        }
    }
    Ok(())
}

fn conflicting_clear_intent(app_root: &Path) -> Result<bool, RestoreArchiveError> {
    let clear_root = app_root.join(MAINTENANCE_DIRECTORY).join("clear-all");
    if !path_exists(&clear_root).map_err(|_| restore_intent_conflict())? {
        return Ok(false);
    }
    let metadata = fs::symlink_metadata(&clear_root).map_err(|_| restore_intent_conflict())?;
    if !metadata.is_dir() || metadata_is_reparse(&metadata) {
        return Err(restore_intent_conflict());
    }
    let intents = clear_root.join(INTENTS_DIRECTORY);
    if !path_exists(&intents).map_err(|_| restore_intent_conflict())? {
        return Ok(false);
    }
    let metadata = fs::symlink_metadata(&intents).map_err(|_| restore_intent_conflict())?;
    if !metadata.is_dir() || metadata_is_reparse(&metadata) {
        return Err(restore_intent_conflict());
    }
    Ok(fs::read_dir(intents)
        .map_err(|_| restore_intent_conflict())?
        .next()
        .is_some())
}

fn read_intent(path: &Path) -> Result<RestoreIntent, RestoreArchiveError> {
    require_regular_file(path).map_err(|_| restore_intent_conflict())?;
    let metadata = fs::symlink_metadata(path).map_err(|_| restore_intent_conflict())?;
    if metadata.len() == 0 || metadata.len() > MAX_INTENT_BYTES {
        return Err(restore_intent_conflict());
    }
    let file = fs::File::open(path).map_err(|_| restore_intent_conflict())?;
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(MAX_INTENT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| restore_intent_conflict())?;
    if bytes.len() as u64 != metadata.len() {
        return Err(restore_intent_conflict());
    }
    require_regular_file(path).map_err(|_| restore_intent_conflict())?;
    serde_json::from_slice(&bytes).map_err(|_| restore_intent_conflict())
}

fn relative_path_text(path: &Path) -> Result<String, RestoreArchiveError> {
    if path.as_os_str().is_empty() {
        return Ok(String::new());
    }
    let mut components = Vec::new();
    for component in path.components() {
        let value = match component {
            std::path::Component::Normal(value) => value
                .to_str()
                .filter(|value| !value.is_empty())
                .ok_or_else(restore_archive_invalid)?,
            _ => return Err(restore_archive_invalid()),
        };
        components.push(value);
    }
    Ok(components.join("/"))
}

fn parse_canonical_uuid(value: &str) -> Option<Uuid> {
    let id = Uuid::parse_str(value).ok()?;
    (id.to_string() == value).then_some(id)
}

fn checkpoint(
    fault: &dyn RestoreFaultInjector,
    boundary: RestoreBoundary,
) -> Result<(), RestoreArchiveError> {
    fault
        .checkpoint(boundary)
        .map_err(|_| restore_recovery_failed())
}

fn map_archive_source_error(error: BackupArchiveError) -> RestoreArchiveError {
    match error.code {
        MaintenanceErrorCode::BackupLimitExceeded => restore_archive_invalid(),
        MaintenanceErrorCode::BackupWriteFailed => restore_stage_failed(),
        _ => restore_archive_invalid(),
    }
}

fn validate_restore_source_path(
    archive: &Path,
    app_root: &Path,
) -> Result<(), RestoreArchiveError> {
    if archive.as_os_str().is_empty()
        || !archive.is_absolute()
        || archive.components().any(|component| {
            matches!(
                component,
                std::path::Component::CurDir | std::path::Component::ParentDir
            )
        })
        || archive
            .extension()
            .and_then(|extension| extension.to_str())
            .is_none_or(|extension| !extension.eq_ignore_ascii_case("tlbackup"))
        || archive.starts_with(app_root)
    {
        return Err(restore_source_invalid());
    }
    let metadata = fs::symlink_metadata(archive).map_err(|_| restore_source_invalid())?;
    if !metadata.is_file() || metadata_is_reparse(&metadata) {
        return Err(restore_source_invalid());
    }
    Ok(())
}

const fn restore_source_invalid() -> RestoreArchiveError {
    RestoreArchiveError::new(MaintenanceErrorCode::RestoreSourceInvalid)
}

const fn restore_archive_invalid() -> RestoreArchiveError {
    RestoreArchiveError::new(MaintenanceErrorCode::RestoreArchiveInvalid)
}

const fn restore_preflight_failed() -> RestoreArchiveError {
    RestoreArchiveError::new(MaintenanceErrorCode::RestorePreflightFailed)
}

const fn restore_intent_conflict() -> RestoreArchiveError {
    RestoreArchiveError::new(MaintenanceErrorCode::RestoreIntentConflict)
}

const fn restore_stage_failed() -> RestoreArchiveError {
    RestoreArchiveError::new(MaintenanceErrorCode::RestoreStageFailed)
}

const fn restore_recovery_failed() -> RestoreArchiveError {
    RestoreArchiveError::new(MaintenanceErrorCode::RestoreRecoveryFailed)
}

#[cfg(test)]
#[path = "restore_test.rs"]
mod tests;
