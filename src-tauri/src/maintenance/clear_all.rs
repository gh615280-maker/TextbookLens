use std::{
    collections::HashSet,
    fmt, fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
};

use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::{
    app_state::AppPaths,
    credentials::CredentialStore,
    db::providers::{credential_key, try_provider_mutation},
    domain::{
        ClearAllDataStatusCode, ClearAllDataSummaryDto, MaintenanceErrorCode, MaintenanceErrorDto,
    },
    errors::{AppError, AppErrorCode, AppResult},
    maintenance::{
        gate::{GateAcquireError, MaintenanceGate},
        journal::metadata_is_reparse,
        recovery_fs::{
            OwnedEntryKind, capture_tree, ensure_direct_child_directory, path_exists,
            remove_captured_tree, rename_checked, require_app_root, require_directory_children,
            require_exact_child_directory, require_regular_file, sync,
        },
        storage::StorageLayout,
    },
};

pub const CLEAR_ALL_DATA_CONFIRMATION_PHRASE: &str = "DELETE ALL TEXTBOOKLENS DATA";

const CLEAR_INTENT_FORMAT: &str = "textbooklens.clear-all-intent";
const CLEAR_INTENT_VERSION: u32 = 1;
const MAINTENANCE_DIRECTORY: &str = "maintenance";
const CLEAR_DIRECTORY: &str = "clear-all";
const RESTORE_DIRECTORY: &str = "restore";
const INTENTS_DIRECTORY: &str = "intents";
const CREDENTIALS_PENDING_DIRECTORY: &str = "credentials-pending";
const READY_DIRECTORY: &str = "ready-to-clear";
const LOCAL_STAGED_DIRECTORY: &str = "local-staged";
const TRASH_DIRECTORY: &str = "trash";
const MAX_INTENT_BYTES: u64 = 32 * 1024 * 1024;
const MAX_CREDENTIAL_TARGETS: usize = 200_000;
const MAX_CONTROL_ENTRIES: usize = 64;
const PROVIDER_KEY_PREFIX: &str = "textbooklens/";
const REMOTE_REFERENCE_PREFIX: &str = "enc:v1:keyring:";
const REMOTE_KEY_PREFIX: &str = "textbooklens/remote-resource/";
const SQLITE_SIDECARS: [&str; 3] = [
    "library.sqlite3-wal",
    "library.sqlite3-shm",
    "library.sqlite3-journal",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClearBoundary {
    IntentWritten,
    BeforeCredentialDelete(usize),
    AfterCredentialDelete(usize),
    CredentialsCleared,
    BeforeLocalMove(usize),
    AfterLocalMove(usize),
    LocalStateStaged,
    BeforeTrashCleanup,
    BeforeIntentCleanup,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClearInjectedCrash;

pub trait ClearFaultInjector: Send + Sync {
    fn checkpoint(&self, boundary: ClearBoundary) -> Result<(), ClearInjectedCrash>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct NoClearFault;

impl ClearFaultInjector for NoClearFault {
    fn checkpoint(&self, _boundary: ClearBoundary) -> Result<(), ClearInjectedCrash> {
        Ok(())
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ClearAllError {
    pub code: MaintenanceErrorCode,
}

impl ClearAllError {
    const fn new(code: MaintenanceErrorCode) -> Self {
        Self { code }
    }
}

impl fmt::Debug for ClearAllError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ClearAllError")
            .field("code", &self.code)
            .finish()
    }
}

impl fmt::Display for ClearAllError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:?}", self.code)
    }
}

impl std::error::Error for ClearAllError {}

#[derive(PartialEq, Eq)]
pub enum ClearServiceError {
    Gate(GateAcquireError),
    Clear(ClearAllError),
}

impl fmt::Debug for ClearServiceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Gate(error) => formatter.debug_tuple("Gate").field(error).finish(),
            Self::Clear(error) => formatter.debug_tuple("Clear").field(error).finish(),
        }
    }
}

impl fmt::Display for ClearServiceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Gate(_) => formatter.write_str("clear maintenance gate unavailable"),
            Self::Clear(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for ClearServiceError {}

impl From<GateAcquireError> for ClearServiceError {
    fn from(error: GateAcquireError) -> Self {
        Self::Gate(error)
    }
}

impl From<ClearAllError> for ClearServiceError {
    fn from(error: ClearAllError) -> Self {
        Self::Clear(error)
    }
}

impl From<ClearServiceError> for MaintenanceErrorDto {
    fn from(error: ClearServiceError) -> Self {
        match error {
            ClearServiceError::Gate(error) => Self::from(error),
            ClearServiceError::Clear(error) => Self::new(error.code),
        }
    }
}

#[derive(Clone)]
pub struct ClearAllDataService {
    pool: SqlitePool,
    paths: AppPaths,
    gate: MaintenanceGate,
    credentials: Arc<dyn CredentialStore>,
    fault: Arc<dyn ClearFaultInjector>,
}

impl ClearAllDataService {
    pub fn new(
        pool: SqlitePool,
        paths: AppPaths,
        gate: MaintenanceGate,
        credentials: Arc<dyn CredentialStore>,
    ) -> Self {
        Self {
            pool,
            paths,
            gate,
            credentials,
            fault: Arc::new(NoClearFault),
        }
    }

    #[doc(hidden)]
    pub fn with_fault_injector(
        pool: SqlitePool,
        paths: AppPaths,
        gate: MaintenanceGate,
        credentials: Arc<dyn CredentialStore>,
        fault: Arc<dyn ClearFaultInjector>,
    ) -> Self {
        Self {
            pool,
            paths,
            gate,
            credentials,
            fault,
        }
    }

    pub async fn clear_all_data(
        &self,
        confirmation: String,
    ) -> Result<ClearAllDataSummaryDto, ClearServiceError> {
        require_confirmation(&confirmation)?;
        let _exclusive = self.gate.try_acquire_maintenance()?;
        StorageLayout::from_app_paths(&self.paths).map_err(|_| clear_root_invalid())?;
        let root = require_app_root(&self.paths.root).map_err(|_| clear_root_invalid())?;
        let store = ClearStore::open_or_create(&root)?;
        store.require_no_restore_conflict()?;
        let _provider_guard = try_provider_mutation(&self.pool)
            .await
            .map_err(|_| clear_intent_conflict())?;

        let mut pending = match store.single_intent()? {
            Some(pending) => pending,
            None => {
                store.require_empty_trash()?;
                let targets =
                    collect_credential_targets(&self.pool, self.credentials.as_ref()).await?;
                let intent = ClearIntent::new(Uuid::new_v4(), targets)?;
                let pending = store.write_credentials_pending(&intent)?;
                checkpoint(self.fault.as_ref(), ClearBoundary::IntentWritten)?;
                pending
            }
        };
        store.validate_pending_trash(&pending)?;
        if pending.phase != ClearPhase::CredentialsPending {
            self.gate.shutdown();
            return Ok(ClearAllDataSummaryDto {
                status: ClearAllDataStatusCode::ReadyToRestart,
                restart_required: true,
            });
        }
        if !delete_credentials(
            self.credentials.as_ref(),
            &pending.intent.targets,
            self.fault.as_ref(),
        )
        .await?
        {
            return Ok(ClearAllDataSummaryDto {
                status: ClearAllDataStatusCode::CredentialCleanupRequired,
                restart_required: false,
            });
        }
        store.transition(&mut pending, ClearPhase::Ready)?;
        checkpoint(self.fault.as_ref(), ClearBoundary::CredentialsCleared)?;
        self.gate.shutdown();
        Ok(ClearAllDataSummaryDto {
            status: ClearAllDataStatusCode::ReadyToRestart,
            restart_required: true,
        })
    }
}

impl fmt::Debug for ClearAllDataService {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ClearAllDataService(<redacted>)")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ClearPhase {
    CredentialsPending,
    Ready,
    LocalStaged,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(transparent)]
struct CredentialTarget(String);

impl CredentialTarget {
    fn new(value: String) -> Result<Self, ClearAllError> {
        let suffix = value
            .strip_prefix(PROVIDER_KEY_PREFIX)
            .ok_or_else(clear_intent_conflict)?;
        let id = match suffix.strip_prefix("remote-resource/") {
            Some(id) => id,
            None if !suffix.contains('/') => suffix,
            None => return Err(clear_intent_conflict()),
        };
        if parse_canonical_uuid(id).is_none() {
            return Err(clear_intent_conflict());
        }
        Ok(Self(value))
    }

    fn value(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for CredentialTarget {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("CredentialTarget(<redacted>)")
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ClearIntent {
    format: String,
    version: u32,
    operation_id: Uuid,
    targets: Vec<CredentialTarget>,
}

impl ClearIntent {
    fn new(operation_id: Uuid, mut targets: Vec<CredentialTarget>) -> Result<Self, ClearAllError> {
        targets.sort_by(|left, right| left.value().cmp(right.value()));
        targets.dedup_by(|left, right| left.value() == right.value());
        if operation_id.is_nil() || targets.len() > MAX_CREDENTIAL_TARGETS {
            return Err(clear_intent_conflict());
        }
        Ok(Self {
            format: CLEAR_INTENT_FORMAT.to_owned(),
            version: CLEAR_INTENT_VERSION,
            operation_id,
            targets,
        })
    }

    fn validate(&self, expected_id: Uuid) -> Result<(), ClearAllError> {
        if self.format != CLEAR_INTENT_FORMAT
            || self.version != CLEAR_INTENT_VERSION
            || self.operation_id != expected_id
            || self.operation_id.is_nil()
            || self.targets.len() > MAX_CREDENTIAL_TARGETS
        {
            return Err(clear_intent_conflict());
        }
        let mut previous: Option<&str> = None;
        for target in &self.targets {
            CredentialTarget::new(target.0.clone())?;
            if previous.is_some_and(|value| value >= target.value()) {
                return Err(clear_intent_conflict());
            }
            previous = Some(target.value());
        }
        Ok(())
    }
}

impl fmt::Debug for ClearIntent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ClearIntent")
            .field("operation_id", &"<redacted>")
            .field("target_count", &self.targets.len())
            .finish()
    }
}

struct PendingClear {
    phase: ClearPhase,
    intent_path: PathBuf,
    intent: ClearIntent,
}

struct ClearStore {
    app_root: PathBuf,
    credentials_pending: PathBuf,
    ready: PathBuf,
    local_staged: PathBuf,
    trash: PathBuf,
}

impl fmt::Debug for ClearStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ClearStore(<redacted>)")
    }
}

impl ClearStore {
    fn open_or_create(app_root: &Path) -> Result<Self, ClearAllError> {
        let app_root = require_app_root(app_root).map_err(|_| clear_root_invalid())?;
        let maintenance = ensure_direct_child_directory(&app_root, MAINTENANCE_DIRECTORY)
            .map_err(|_| clear_root_invalid())?;
        let clear_root = ensure_direct_child_directory(&maintenance, CLEAR_DIRECTORY)
            .map_err(|_| clear_root_invalid())?;
        let intents = ensure_direct_child_directory(&clear_root, INTENTS_DIRECTORY)
            .map_err(|_| clear_root_invalid())?;
        let credentials_pending =
            ensure_direct_child_directory(&intents, CREDENTIALS_PENDING_DIRECTORY)
                .map_err(|_| clear_root_invalid())?;
        let ready = ensure_direct_child_directory(&intents, READY_DIRECTORY)
            .map_err(|_| clear_root_invalid())?;
        let local_staged = ensure_direct_child_directory(&intents, LOCAL_STAGED_DIRECTORY)
            .map_err(|_| clear_root_invalid())?;
        let trash = ensure_direct_child_directory(&clear_root, TRASH_DIRECTORY)
            .map_err(|_| clear_root_invalid())?;
        let store = Self {
            app_root,
            credentials_pending,
            ready,
            local_staged,
            trash,
        };
        store.validate_static_control_shape()?;
        Ok(store)
    }

    fn open_existing(app_root: &Path) -> Result<Option<Self>, ClearAllError> {
        let app_root = require_app_root(app_root).map_err(|_| clear_recovery_failed())?;
        let maintenance = app_root.join(MAINTENANCE_DIRECTORY);
        if !path_exists(&maintenance).map_err(|_| clear_recovery_failed())? {
            return Ok(None);
        }
        require_exact_child_directory(&app_root, &maintenance)
            .map_err(|_| clear_recovery_failed())?;
        let clear_root = maintenance.join(CLEAR_DIRECTORY);
        if !path_exists(&clear_root).map_err(|_| clear_recovery_failed())? {
            return Ok(None);
        }
        require_exact_child_directory(&maintenance, &clear_root)
            .map_err(|_| clear_recovery_failed())?;
        let intents = clear_root.join(INTENTS_DIRECTORY);
        let credentials_pending = intents.join(CREDENTIALS_PENDING_DIRECTORY);
        let ready = intents.join(READY_DIRECTORY);
        let local_staged = intents.join(LOCAL_STAGED_DIRECTORY);
        let trash = clear_root.join(TRASH_DIRECTORY);
        for (parent, child) in [
            (&clear_root, &intents),
            (&intents, &credentials_pending),
            (&intents, &ready),
            (&intents, &local_staged),
            (&clear_root, &trash),
        ] {
            require_exact_child_directory(parent, child).map_err(|_| clear_recovery_failed())?;
        }
        let store = Self {
            app_root,
            credentials_pending,
            ready,
            local_staged,
            trash,
        };
        store.validate_static_control_shape()?;
        Ok(Some(store))
    }

    fn validate_static_control_shape(&self) -> Result<(), ClearAllError> {
        let clear_root = self.trash.parent().ok_or_else(clear_intent_conflict)?;
        let intents = self
            .credentials_pending
            .parent()
            .ok_or_else(clear_intent_conflict)?;
        require_directory_children(clear_root, &["intents", "trash"], true)
            .map_err(|_| clear_intent_conflict())?;
        require_directory_children(
            intents,
            &[
                CREDENTIALS_PENDING_DIRECTORY,
                READY_DIRECTORY,
                LOCAL_STAGED_DIRECTORY,
            ],
            true,
        )
        .map_err(|_| clear_intent_conflict())?;
        Ok(())
    }

    fn validate_pending_trash(&self, pending: &PendingClear) -> Result<(), ClearAllError> {
        let operation_name = pending.intent.operation_id.to_string();
        let children = require_directory_children(&self.trash, &[operation_name.as_str()], false)
            .map_err(|_| clear_intent_conflict())?;
        if pending.phase == ClearPhase::CredentialsPending && !children.is_empty() {
            return Err(clear_intent_conflict());
        }
        if children.len() == 1 {
            self.validate_trash_operation(&self.trash.join(operation_name))?;
        }
        Ok(())
    }

    fn validate_trash_operation(&self, operation: &Path) -> Result<(), ClearAllError> {
        let allowed = HashSet::from([
            "library.sqlite3",
            "library.sqlite3-wal",
            "library.sqlite3-shm",
            "library.sqlite3-journal",
            "books",
            "cache",
            "logs",
            "restore-control",
        ]);
        let mut seen = HashSet::new();
        for entry in fs::read_dir(operation).map_err(|_| clear_intent_conflict())? {
            let entry = entry.map_err(|_| clear_intent_conflict())?;
            let name = entry
                .file_name()
                .to_str()
                .filter(|name| allowed.contains(*name))
                .ok_or_else(clear_intent_conflict)?
                .to_owned();
            if entry.path().parent() != Some(operation) || !seen.insert(name.clone()) {
                return Err(clear_intent_conflict());
            }
            let kind = if name.starts_with("library.sqlite3") {
                require_regular_file(&entry.path()).map_err(|_| clear_intent_conflict())?;
                OwnedEntryKind::File
            } else {
                require_exact_child_directory(operation, &entry.path())
                    .map_err(|_| clear_intent_conflict())?;
                OwnedEntryKind::Directory
            };
            capture_tree(&self.app_root, &entry.path(), kind)
                .map_err(|_| clear_intent_conflict())?;
        }
        Ok(())
    }

    fn require_no_restore_conflict(&self) -> Result<(), ClearAllError> {
        let restore = self
            .app_root
            .join(MAINTENANCE_DIRECTORY)
            .join(RESTORE_DIRECTORY);
        if !path_exists(&restore).map_err(|_| clear_intent_conflict())? {
            return Ok(());
        }
        let metadata = fs::symlink_metadata(&restore).map_err(|_| clear_intent_conflict())?;
        if !metadata.is_dir() || metadata_is_reparse(&metadata) {
            return Err(clear_intent_conflict());
        }
        let intents = restore.join(INTENTS_DIRECTORY);
        if !path_exists(&intents).map_err(|_| clear_intent_conflict())? {
            return Ok(());
        }
        if tree_has_file(&intents)? {
            Err(clear_intent_conflict())
        } else {
            Ok(())
        }
    }

    fn require_empty_trash(&self) -> Result<(), ClearAllError> {
        if fs::read_dir(&self.trash)
            .map_err(|_| clear_intent_conflict())?
            .next()
            .is_some()
        {
            Err(clear_intent_conflict())
        } else {
            Ok(())
        }
    }

    fn write_credentials_pending(
        &self,
        intent: &ClearIntent,
    ) -> Result<PendingClear, ClearAllError> {
        let target = self.intent_path(ClearPhase::CredentialsPending, intent.operation_id);
        if path_exists(&target).map_err(|_| clear_intent_conflict())? {
            return Err(clear_intent_conflict());
        }
        let temporary = self
            .credentials_pending
            .join(format!(".clear-{}.tmp", intent.operation_id));
        let bytes = serde_json::to_vec(intent).map_err(|_| clear_intent_conflict())?;
        if bytes.is_empty() || bytes.len() > MAX_INTENT_BYTES as usize {
            return Err(clear_intent_conflict());
        }
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|_| clear_intent_conflict())?;
        let write = file
            .write_all(&bytes)
            .and_then(|()| file.sync_all())
            .map_err(|_| clear_intent_conflict());
        drop(file);
        if let Err(error) = write {
            let _ = fs::remove_file(&temporary);
            return Err(error);
        }
        rename_checked(&self.app_root, &temporary, &target, OwnedEntryKind::File)
            .map_err(|_| clear_intent_conflict())?;
        Ok(PendingClear {
            phase: ClearPhase::CredentialsPending,
            intent_path: target,
            intent: intent.clone(),
        })
    }

    fn transition(
        &self,
        pending: &mut PendingClear,
        next: ClearPhase,
    ) -> Result<(), ClearAllError> {
        let target = self.intent_path(next, pending.intent.operation_id);
        rename_checked(
            &self.app_root,
            &pending.intent_path,
            &target,
            OwnedEntryKind::File,
        )
        .map_err(|_| clear_recovery_failed())?;
        pending.intent_path = target;
        pending.phase = next;
        Ok(())
    }

    fn intent_path(&self, phase: ClearPhase, operation_id: Uuid) -> PathBuf {
        let parent = match phase {
            ClearPhase::CredentialsPending => &self.credentials_pending,
            ClearPhase::Ready => &self.ready,
            ClearPhase::LocalStaged => &self.local_staged,
        };
        parent.join(format!("{operation_id}.json"))
    }

    fn single_intent(&self) -> Result<Option<PendingClear>, ClearAllError> {
        let mut pending = Vec::new();
        for (phase, directory) in [
            (ClearPhase::CredentialsPending, &self.credentials_pending),
            (ClearPhase::Ready, &self.ready),
            (ClearPhase::LocalStaged, &self.local_staged),
        ] {
            for entry in fs::read_dir(directory).map_err(|_| clear_intent_conflict())? {
                if pending.len() >= MAX_CONTROL_ENTRIES {
                    return Err(clear_intent_conflict());
                }
                let entry = entry.map_err(|_| clear_intent_conflict())?;
                let path = entry.path();
                if path.parent() != Some(directory.as_path()) {
                    return Err(clear_intent_conflict());
                }
                let file_name = entry.file_name();
                let name = file_name.to_str().ok_or_else(clear_intent_conflict)?;
                if name.starts_with(".clear-") && name.ends_with(".tmp") {
                    return Err(clear_intent_conflict());
                }
                let id = Path::new(name)
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .and_then(parse_canonical_uuid)
                    .filter(|_| {
                        Path::new(name).extension().and_then(|ext| ext.to_str()) == Some("json")
                    })
                    .ok_or_else(clear_intent_conflict)?;
                let intent = read_intent(&path)?;
                intent.validate(id)?;
                pending.push(PendingClear {
                    phase,
                    intent_path: path,
                    intent,
                });
            }
        }
        if pending.len() > 1 {
            return Err(clear_intent_conflict());
        }
        Ok(pending.pop())
    }

    fn trash_root(&self, operation_id: Uuid) -> PathBuf {
        self.trash.join(operation_id.to_string())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClearStartupOutcome {
    NoPending,
    CredentialCleanupRequired,
    Cleared,
}

pub async fn recover_pending_clear(
    app_root: &Path,
    credentials: &dyn CredentialStore,
) -> AppResult<ClearStartupOutcome> {
    recover_pending_clear_with_fault(app_root, credentials, &NoClearFault)
        .await
        .map_err(|_| AppError::new(AppErrorCode::RequestConflict))
}

#[doc(hidden)]
pub async fn recover_pending_clear_with_fault(
    app_root: &Path,
    credentials: &dyn CredentialStore,
    fault: &dyn ClearFaultInjector,
) -> Result<ClearStartupOutcome, ClearAllError> {
    if cleanup_empty_clear_control_if_present(app_root)? {
        return Ok(ClearStartupOutcome::NoPending);
    }
    let Some(store) = ClearStore::open_existing(app_root)? else {
        return Ok(ClearStartupOutcome::NoPending);
    };
    store.require_no_restore_conflict()?;
    let Some(mut pending) = store.single_intent()? else {
        store.require_empty_trash()?;
        return Ok(ClearStartupOutcome::NoPending);
    };
    store.validate_pending_trash(&pending)?;
    if pending.phase == ClearPhase::CredentialsPending {
        if !delete_credentials(credentials, &pending.intent.targets, fault).await? {
            return Ok(ClearStartupOutcome::CredentialCleanupRequired);
        }
        store.transition(&mut pending, ClearPhase::Ready)?;
        checkpoint(fault, ClearBoundary::CredentialsCleared)?;
    }
    let trash_root = store.trash_root(pending.intent.operation_id);
    if pending.phase == ClearPhase::Ready {
        stage_local_data(&store, &trash_root, fault)?;
        store.transition(&mut pending, ClearPhase::LocalStaged)?;
        checkpoint(fault, ClearBoundary::LocalStateStaged)?;
    }
    if pending.phase == ClearPhase::LocalStaged {
        validate_cleared_root(&store)?;
        checkpoint(fault, ClearBoundary::BeforeTrashCleanup)?;
        if path_exists(&trash_root).map_err(|_| clear_recovery_failed())? {
            let snapshot = capture_tree(&store.app_root, &trash_root, OwnedEntryKind::Directory)
                .map_err(|_| clear_recovery_failed())?;
            remove_captured_tree(&store.app_root, &snapshot)
                .map_err(|_| clear_recovery_failed())?;
        }
        checkpoint(fault, ClearBoundary::BeforeIntentCleanup)?;
        require_regular_file(&pending.intent_path).map_err(|_| clear_recovery_failed())?;
        fs::remove_file(&pending.intent_path).map_err(|_| clear_recovery_failed())?;
        sync(
            pending
                .intent_path
                .parent()
                .ok_or_else(clear_recovery_failed)?,
        )
        .map_err(|_| clear_recovery_failed())?;
        cleanup_empty_clear_control(&store)?;
    }
    Ok(ClearStartupOutcome::Cleared)
}

fn cleanup_empty_clear_control_if_present(app_root: &Path) -> Result<bool, ClearAllError> {
    let root = require_app_root(app_root).map_err(|_| clear_recovery_failed())?;
    let clear_root = root.join(MAINTENANCE_DIRECTORY).join(CLEAR_DIRECTORY);
    if !path_exists(&clear_root).map_err(|_| clear_recovery_failed())? {
        return Ok(false);
    }
    if tree_has_file(&clear_root)? {
        return Ok(false);
    }
    validate_partial_empty_clear_control(&clear_root)?;
    let snapshot = capture_tree(&root, &clear_root, OwnedEntryKind::Directory)
        .map_err(|_| clear_recovery_failed())?;
    remove_captured_tree(&root, &snapshot).map_err(|_| clear_recovery_failed())?;
    remove_empty_maintenance(&root)?;
    Ok(true)
}

fn validate_partial_empty_clear_control(clear_root: &Path) -> Result<(), ClearAllError> {
    let children =
        require_directory_children(clear_root, &[INTENTS_DIRECTORY, TRASH_DIRECTORY], false)
            .map_err(|_| clear_intent_conflict())?;
    if children.iter().any(|name| name == INTENTS_DIRECTORY) {
        let intents = clear_root.join(INTENTS_DIRECTORY);
        let phases = require_directory_children(
            &intents,
            &[
                CREDENTIALS_PENDING_DIRECTORY,
                READY_DIRECTORY,
                LOCAL_STAGED_DIRECTORY,
            ],
            false,
        )
        .map_err(|_| clear_intent_conflict())?;
        for phase in phases {
            require_directory_children(&intents.join(phase), &[], true)
                .map_err(|_| clear_intent_conflict())?;
        }
    }
    if children.iter().any(|name| name == TRASH_DIRECTORY) {
        require_directory_children(&clear_root.join(TRASH_DIRECTORY), &[], true)
            .map_err(|_| clear_intent_conflict())?;
    }
    Ok(())
}

fn cleanup_empty_clear_control(store: &ClearStore) -> Result<(), ClearAllError> {
    let clear_root = store.trash.parent().ok_or_else(clear_recovery_failed)?;
    if tree_has_file(clear_root)? {
        return Err(clear_recovery_failed());
    }
    let snapshot = capture_tree(&store.app_root, clear_root, OwnedEntryKind::Directory)
        .map_err(|_| clear_recovery_failed())?;
    remove_captured_tree(&store.app_root, &snapshot).map_err(|_| clear_recovery_failed())?;
    remove_empty_maintenance(&store.app_root)
}

fn remove_empty_maintenance(app_root: &Path) -> Result<(), ClearAllError> {
    let maintenance = app_root.join(MAINTENANCE_DIRECTORY);
    if path_exists(&maintenance).map_err(|_| clear_recovery_failed())?
        && fs::read_dir(&maintenance)
            .map_err(|_| clear_recovery_failed())?
            .next()
            .is_none()
    {
        fs::remove_dir(&maintenance).map_err(|_| clear_recovery_failed())?;
        sync(app_root).map_err(|_| clear_recovery_failed())?;
    }
    Ok(())
}

fn stage_local_data(
    store: &ClearStore,
    trash_root: &Path,
    fault: &dyn ClearFaultInjector,
) -> Result<(), ClearAllError> {
    validate_clear_root_entries(store)?;
    if !path_exists(trash_root).map_err(|_| clear_recovery_failed())? {
        ensure_direct_child_directory(&store.trash, &operation_name(trash_root)?)
            .map_err(|_| clear_recovery_failed())?;
    }
    require_exact_child_directory(&store.trash, trash_root).map_err(|_| clear_recovery_failed())?;
    let mut units = vec![
        ("library.sqlite3", OwnedEntryKind::File, true),
        ("books", OwnedEntryKind::Directory, true),
        ("cache", OwnedEntryKind::Directory, true),
        ("logs", OwnedEntryKind::Directory, true),
    ];
    units.extend(
        SQLITE_SIDECARS
            .into_iter()
            .map(|name| (name, OwnedEntryKind::File, false)),
    );
    for (index, (name, kind, required)) in units.iter().enumerate() {
        let source = store.app_root.join(name);
        let destination = trash_root.join(name);
        match (
            path_exists(&source).map_err(|_| clear_recovery_failed())?,
            path_exists(&destination).map_err(|_| clear_recovery_failed())?,
            required,
        ) {
            (true, false, _) => {
                checkpoint(fault, ClearBoundary::BeforeLocalMove(index))?;
                capture_tree(&store.app_root, &source, *kind)
                    .map_err(|_| clear_recovery_failed())?;
                rename_checked(&store.app_root, &source, &destination, *kind)
                    .map_err(|_| clear_recovery_failed())?;
                checkpoint(fault, ClearBoundary::AfterLocalMove(index))?;
            }
            (false, true, _) => {}
            (false, false, false) => {}
            _ => return Err(clear_recovery_failed()),
        }
    }
    let restore = store
        .app_root
        .join(MAINTENANCE_DIRECTORY)
        .join(RESTORE_DIRECTORY);
    let restore_trash = trash_root.join("restore-control");
    match (
        path_exists(&restore).map_err(|_| clear_recovery_failed())?,
        path_exists(&restore_trash).map_err(|_| clear_recovery_failed())?,
    ) {
        (true, false) => {
            let index = units.len();
            checkpoint(fault, ClearBoundary::BeforeLocalMove(index))?;
            capture_tree(&store.app_root, &restore, OwnedEntryKind::Directory)
                .map_err(|_| clear_recovery_failed())?;
            rename_checked(
                &store.app_root,
                &restore,
                &restore_trash,
                OwnedEntryKind::Directory,
            )
            .map_err(|_| clear_recovery_failed())?;
            checkpoint(fault, ClearBoundary::AfterLocalMove(index))?;
        }
        (false, true) | (false, false) => {}
        (true, true) => return Err(clear_recovery_failed()),
    }
    validate_cleared_root(store)
}

fn validate_clear_root_entries(store: &ClearStore) -> Result<(), ClearAllError> {
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
    for entry in fs::read_dir(&store.app_root).map_err(|_| clear_root_invalid())? {
        let entry = entry.map_err(|_| clear_root_invalid())?;
        let file_name = entry.file_name();
        let name = file_name.to_str().ok_or_else(clear_root_invalid)?;
        if name != name.to_ascii_lowercase() || !allowed.contains(name) {
            return Err(clear_root_invalid());
        }
        let metadata = fs::symlink_metadata(entry.path()).map_err(|_| clear_root_invalid())?;
        if metadata_is_reparse(&metadata) || (!metadata.is_file() && !metadata.is_dir()) {
            return Err(clear_root_invalid());
        }
    }
    let maintenance = store.app_root.join(MAINTENANCE_DIRECTORY);
    for entry in fs::read_dir(&maintenance).map_err(|_| clear_root_invalid())? {
        let entry = entry.map_err(|_| clear_root_invalid())?;
        let file_name = entry.file_name();
        let name = file_name.to_str().ok_or_else(clear_root_invalid)?;
        if !matches!(name, CLEAR_DIRECTORY | RESTORE_DIRECTORY) {
            return Err(clear_root_invalid());
        }
        let metadata = fs::symlink_metadata(entry.path()).map_err(|_| clear_root_invalid())?;
        if !metadata.is_dir() || metadata_is_reparse(&metadata) {
            return Err(clear_root_invalid());
        }
    }
    Ok(())
}

fn validate_cleared_root(store: &ClearStore) -> Result<(), ClearAllError> {
    for name in ["library.sqlite3", "books", "cache", "logs"]
        .into_iter()
        .chain(SQLITE_SIDECARS)
    {
        if path_exists(&store.app_root.join(name)).map_err(|_| clear_recovery_failed())? {
            return Err(clear_recovery_failed());
        }
    }
    let maintenance = store.app_root.join(MAINTENANCE_DIRECTORY);
    for entry in fs::read_dir(&maintenance).map_err(|_| clear_recovery_failed())? {
        let entry = entry.map_err(|_| clear_recovery_failed())?;
        if entry.file_name() != CLEAR_DIRECTORY {
            return Err(clear_recovery_failed());
        }
        let metadata = fs::symlink_metadata(entry.path()).map_err(|_| clear_recovery_failed())?;
        if !metadata.is_dir() || metadata_is_reparse(&metadata) {
            return Err(clear_recovery_failed());
        }
    }
    Ok(())
}

async fn collect_credential_targets(
    pool: &SqlitePool,
    credentials: &dyn CredentialStore,
) -> Result<Vec<CredentialTarget>, ClearAllError> {
    let limit = i64::try_from(MAX_CREDENTIAL_TARGETS + 1).map_err(|_| clear_intent_conflict())?;
    let profiles = sqlx::query("SELECT id FROM provider_profiles ORDER BY id LIMIT ?")
        .bind(limit)
        .fetch_all(pool)
        .await
        .map_err(|_| clear_intent_conflict())?;
    let references = sqlx::query(
        "SELECT encrypted_reference FROM provider_remote_resources ORDER BY id LIMIT ?",
    )
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(|_| clear_intent_conflict())?;
    if profiles.len().saturating_add(references.len()) > MAX_CREDENTIAL_TARGETS {
        return Err(clear_intent_conflict());
    }
    let mut targets = Vec::with_capacity(profiles.len().saturating_add(references.len()));
    for row in profiles {
        let value: String = row.try_get("id").map_err(|_| clear_intent_conflict())?;
        let id = parse_canonical_uuid(&value).ok_or_else(clear_intent_conflict)?;
        targets.push(CredentialTarget::new(credential_key(id))?);
    }
    for row in references {
        let reference: String = row
            .try_get("encrypted_reference")
            .map_err(|_| clear_intent_conflict())?;
        let id = reference
            .strip_prefix(REMOTE_REFERENCE_PREFIX)
            .and_then(parse_canonical_uuid)
            .ok_or_else(clear_intent_conflict)?;
        targets.push(CredentialTarget::new(format!("{REMOTE_KEY_PREFIX}{id}"))?);
    }
    let enumerated = credentials
        .list_textbooklens_keys()
        .await
        .map_err(|_| clear_credential_cleanup_required())?;
    if targets.len().saturating_add(enumerated.len()) > MAX_CREDENTIAL_TARGETS {
        return Err(clear_intent_conflict());
    }
    for key in enumerated {
        targets.push(CredentialTarget::new(key)?);
    }
    Ok(targets)
}

async fn delete_credentials(
    credentials: &dyn CredentialStore,
    targets: &[CredentialTarget],
    fault: &dyn ClearFaultInjector,
) -> Result<bool, ClearAllError> {
    let mut complete = true;
    for (index, target) in targets.iter().enumerate() {
        checkpoint(fault, ClearBoundary::BeforeCredentialDelete(index))?;
        if credentials.delete(target.value()).await.is_err() {
            complete = false;
        }
        checkpoint(fault, ClearBoundary::AfterCredentialDelete(index))?;
    }
    Ok(complete)
}

fn require_confirmation(value: &str) -> Result<(), ClearAllError> {
    if value.as_bytes() == CLEAR_ALL_DATA_CONFIRMATION_PHRASE.as_bytes() {
        Ok(())
    } else {
        Err(clear_confirmation_required())
    }
}

fn read_intent(path: &Path) -> Result<ClearIntent, ClearAllError> {
    require_regular_file(path).map_err(|_| clear_intent_conflict())?;
    let metadata = fs::symlink_metadata(path).map_err(|_| clear_intent_conflict())?;
    if metadata.len() == 0 || metadata.len() > MAX_INTENT_BYTES {
        return Err(clear_intent_conflict());
    }
    let file = fs::File::open(path).map_err(|_| clear_intent_conflict())?;
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(MAX_INTENT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| clear_intent_conflict())?;
    if bytes.len() as u64 != metadata.len() {
        return Err(clear_intent_conflict());
    }
    require_regular_file(path).map_err(|_| clear_intent_conflict())?;
    serde_json::from_slice(&bytes).map_err(|_| clear_intent_conflict())
}

fn tree_has_file(root: &Path) -> Result<bool, ClearAllError> {
    let mut pending = vec![root.to_path_buf()];
    let mut count = 0_usize;
    while let Some(directory) = pending.pop() {
        count = count.checked_add(1).ok_or_else(clear_intent_conflict)?;
        if count > MAX_CONTROL_ENTRIES {
            return Err(clear_intent_conflict());
        }
        for entry in fs::read_dir(directory).map_err(|_| clear_intent_conflict())? {
            let entry = entry.map_err(|_| clear_intent_conflict())?;
            let metadata =
                fs::symlink_metadata(entry.path()).map_err(|_| clear_intent_conflict())?;
            if metadata_is_reparse(&metadata) {
                return Err(clear_intent_conflict());
            }
            if metadata.is_file() {
                return Ok(true);
            }
            if metadata.is_dir() {
                pending.push(entry.path());
            } else {
                return Err(clear_intent_conflict());
            }
        }
    }
    Ok(false)
}

fn operation_name(path: &Path) -> Result<String, ClearAllError> {
    path.file_name()
        .and_then(|name| name.to_str())
        .and_then(parse_canonical_uuid)
        .map(|id| id.to_string())
        .ok_or_else(clear_recovery_failed)
}

fn parse_canonical_uuid(value: &str) -> Option<Uuid> {
    let id = Uuid::parse_str(value).ok()?;
    (id.to_string() == value).then_some(id)
}

fn checkpoint(
    fault: &dyn ClearFaultInjector,
    boundary: ClearBoundary,
) -> Result<(), ClearAllError> {
    fault
        .checkpoint(boundary)
        .map_err(|_| clear_recovery_failed())
}

const fn clear_confirmation_required() -> ClearAllError {
    ClearAllError::new(MaintenanceErrorCode::ClearConfirmationRequired)
}

const fn clear_root_invalid() -> ClearAllError {
    ClearAllError::new(MaintenanceErrorCode::ClearRootInvalid)
}

const fn clear_intent_conflict() -> ClearAllError {
    ClearAllError::new(MaintenanceErrorCode::ClearIntentConflict)
}

const fn clear_recovery_failed() -> ClearAllError {
    ClearAllError::new(MaintenanceErrorCode::ClearRecoveryFailed)
}

const fn clear_credential_cleanup_required() -> ClearAllError {
    ClearAllError::new(MaintenanceErrorCode::ClearCredentialCleanupRequired)
}

#[cfg(test)]
#[path = "clear_all_test.rs"]
mod tests;
