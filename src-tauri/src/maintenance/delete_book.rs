use std::{
    collections::HashSet,
    fmt, fs,
    path::{Path, PathBuf},
};

use sqlx::SqlitePool;
use uuid::Uuid;

use crate::{
    app_state::AppPaths,
    book_repository::{
        self as books, BookDeletePlan, CompleteDeleteError, CompleteDeleteFaultInjector,
        CompleteDeleteInjectedCrash, CompleteDeleteResult, CompleteDeleteStep,
    },
    domain::{BookFormat, ImportStatus},
    errors::{AppError, AppErrorCode, AppResult},
    maintenance::{
        gate::MaintenanceGate,
        journal::{
            DeleteJournal, JournalBoundary, JournalError, JournalFaultInjector,
            JournalInjectedCrash, JournalOperationError, JournalStore, MAX_DELETE_JOURNAL_ENTRIES,
            NoJournalFault, RelativePathToken, metadata_is_reparse, rename_owned_atomic,
            sync_owned_directory,
        },
    },
};

const MAX_DELETE_TREE_ENTRIES: usize = 100_000;
const MAX_DELETE_TREE_DEPTH: usize = 64;
const MAX_DELETE_TREE_FILE_BYTES: u64 = 1 << 40;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DeletePolicy {
    ReadyOrFailed,
    FailedOnly,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DeleteBoundary {
    Journal(JournalBoundary),
    BeforeTrashRootCreate,
    TrashRootCreated,
    BeforeStageMove(usize),
    StageMoved(usize),
    StageSourceParentSynced(usize),
    StageTrashParentSynced(usize),
    StageVerified(usize),
    AfterStageMove(usize),
    Database(CompleteDeleteStep),
    BeforeFinalizeEntry(usize),
    FinalizeEntryRemoved(usize),
    FinalizeEntryParentSynced(usize),
    AfterFinalizeEntry(usize),
    BeforeTrashRootRemove,
    TrashRootRemoved,
}

impl DeleteBoundary {
    pub fn fault_matrix(entry_count: usize) -> Vec<Self> {
        let mut boundaries = JournalBoundary::ALL
            .into_iter()
            .filter(|boundary| {
                !matches!(
                    boundary,
                    JournalBoundary::CommitTempCreated
                        | JournalBoundary::CommitTempWritten
                        | JournalBoundary::CommitTempSynced
                        | JournalBoundary::CommitRenamed
                        | JournalBoundary::CommitParentSynced
                        | JournalBoundary::CommitRemoved
                        | JournalBoundary::CommitRemovalSynced
                        | JournalBoundary::IntentRemoved
                        | JournalBoundary::IntentRemovalSynced
                )
            })
            .map(Self::Journal)
            .collect::<Vec<_>>();
        boundaries.push(Self::BeforeTrashRootCreate);
        boundaries.push(Self::TrashRootCreated);
        for index in 0..entry_count {
            boundaries.push(Self::BeforeStageMove(index));
            boundaries.push(Self::StageMoved(index));
            boundaries.push(Self::StageSourceParentSynced(index));
            boundaries.push(Self::StageTrashParentSynced(index));
            boundaries.push(Self::StageVerified(index));
            boundaries.push(Self::AfterStageMove(index));
        }
        boundaries.extend(CompleteDeleteStep::ALL.into_iter().map(Self::Database));
        boundaries.extend(
            JournalBoundary::ALL
                .into_iter()
                .filter(|boundary| {
                    matches!(
                        boundary,
                        JournalBoundary::CommitTempCreated
                            | JournalBoundary::CommitTempWritten
                            | JournalBoundary::CommitTempSynced
                            | JournalBoundary::CommitRenamed
                            | JournalBoundary::CommitParentSynced
                    )
                })
                .map(Self::Journal),
        );
        for index in 0..entry_count {
            boundaries.push(Self::BeforeFinalizeEntry(index));
            boundaries.push(Self::FinalizeEntryRemoved(index));
            boundaries.push(Self::FinalizeEntryParentSynced(index));
            boundaries.push(Self::AfterFinalizeEntry(index));
        }
        boundaries.push(Self::BeforeTrashRootRemove);
        boundaries.push(Self::TrashRootRemoved);
        boundaries.extend(
            JournalBoundary::ALL
                .into_iter()
                .filter(|boundary| {
                    matches!(
                        boundary,
                        JournalBoundary::CommitRemoved
                            | JournalBoundary::CommitRemovalSynced
                            | JournalBoundary::IntentRemoved
                            | JournalBoundary::IntentRemovalSynced
                    )
                })
                .map(Self::Journal),
        );
        boundaries
    }

    pub fn is_after_database_commit(self) -> bool {
        match self {
            Self::Database(CompleteDeleteStep::AfterCommit)
            | Self::BeforeFinalizeEntry(_)
            | Self::FinalizeEntryRemoved(_)
            | Self::FinalizeEntryParentSynced(_)
            | Self::AfterFinalizeEntry(_)
            | Self::BeforeTrashRootRemove
            | Self::TrashRootRemoved => true,
            Self::Journal(boundary) => matches!(
                boundary,
                JournalBoundary::CommitTempCreated
                    | JournalBoundary::CommitTempWritten
                    | JournalBoundary::CommitTempSynced
                    | JournalBoundary::CommitRenamed
                    | JournalBoundary::CommitParentSynced
                    | JournalBoundary::CommitRemoved
                    | JournalBoundary::CommitRemovalSynced
                    | JournalBoundary::IntentRemoved
                    | JournalBoundary::IntentRemovalSynced
            ),
            _ => false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeleteInjectedCrash;

pub trait DeleteFaultInjector: Send + Sync {
    fn checkpoint(&self, boundary: DeleteBoundary) -> Result<(), DeleteInjectedCrash>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct NoDeleteFault;

impl DeleteFaultInjector for NoDeleteFault {
    fn checkpoint(&self, _boundary: DeleteBoundary) -> Result<(), DeleteInjectedCrash> {
        Ok(())
    }
}

pub enum DeleteBookError {
    App(AppError),
    Injected(DeleteBoundary),
}

impl DeleteBookError {
    pub fn injected_boundary(&self) -> Option<DeleteBoundary> {
        match self {
            Self::Injected(boundary) => Some(*boundary),
            Self::App(_) => None,
        }
    }

    pub fn into_app_error(self) -> AppError {
        match self {
            Self::App(error) => error,
            Self::Injected(_) => AppError::new(AppErrorCode::LocalIoError),
        }
    }
}

impl fmt::Debug for DeleteBookError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::App(error) => formatter
                .debug_struct("DeleteBookError")
                .field("code", &error.code)
                .finish(),
            Self::Injected(boundary) => formatter
                .debug_struct("DeleteBookInjectedCrash")
                .field("boundary", boundary)
                .finish(),
        }
    }
}

impl From<AppError> for DeleteBookError {
    fn from(error: AppError) -> Self {
        Self::App(error)
    }
}

#[derive(Clone)]
pub struct DeleteBookOutcome {
    detached_remote_resource_ids: Vec<Uuid>,
}

impl DeleteBookOutcome {
    pub fn detached_remote_resource_ids(&self) -> &[Uuid] {
        &self.detached_remote_resource_ids
    }

    pub fn detached_remote_resource_count(&self) -> usize {
        self.detached_remote_resource_ids.len()
    }
}

impl fmt::Debug for DeleteBookOutcome {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DeleteBookOutcome")
            .field(
                "detached_remote_resource_count",
                &self.detached_remote_resource_ids.len(),
            )
            .finish()
    }
}

#[derive(Clone)]
pub struct DeleteBookService {
    pool: SqlitePool,
    paths: AppPaths,
    gate: MaintenanceGate,
}

impl DeleteBookService {
    pub fn new(pool: SqlitePool, paths: AppPaths, gate: MaintenanceGate) -> Self {
        Self { pool, paths, gate }
    }

    pub async fn delete_book(&self, book_id: Uuid) -> AppResult<DeleteBookOutcome> {
        self.delete_with_fault(book_id, DeletePolicy::ReadyOrFailed, &NoDeleteFault)
            .await
            .map_err(DeleteBookError::into_app_error)
    }

    pub async fn delete_failed_book(&self, book_id: Uuid) -> AppResult<DeleteBookOutcome> {
        self.delete_with_fault(book_id, DeletePolicy::FailedOnly, &NoDeleteFault)
            .await
            .map_err(DeleteBookError::into_app_error)
    }

    pub async fn delete_book_with_fault(
        &self,
        book_id: Uuid,
        fault: &dyn DeleteFaultInjector,
    ) -> Result<DeleteBookOutcome, DeleteBookError> {
        self.delete_with_fault(book_id, DeletePolicy::ReadyOrFailed, fault)
            .await
    }

    async fn delete_with_fault(
        &self,
        book_id: Uuid,
        policy: DeletePolicy,
        fault: &dyn DeleteFaultInjector,
    ) -> Result<DeleteBookOutcome, DeleteBookError> {
        let _exclusive = self
            .gate
            .try_acquire_maintenance()
            .map_err(|error| AppError::new(error.as_app_error_code()))?;
        recover_pending_deletions_async(&self.pool, &self.paths)
            .await
            .map_err(DeleteBookError::App)?;
        let result = self.execute_delete(book_id, policy, fault).await;
        if matches!(result, Err(DeleteBookError::App(_)))
            && let Err(recovery_error) =
                recover_pending_deletions_async(&self.pool, &self.paths).await
        {
            return Err(DeleteBookError::App(recovery_error));
        }
        result
    }

    async fn execute_delete(
        &self,
        book_id: Uuid,
        policy: DeletePolicy,
        fault: &dyn DeleteFaultInjector,
    ) -> Result<DeleteBookOutcome, DeleteBookError> {
        let plan = books::load_delete_plan(&self.pool, book_id).await?;
        require_terminal_plan(&plan, policy)?;
        let store = JournalStore::open(&self.paths).map_err(journal_app_error)?;
        store
            .cleanup_stale_temporary_files()
            .map_err(journal_app_error)?;
        let targets = collect_delete_targets(&self.paths, &store, &plan)?;
        let journal_id = Uuid::new_v4();
        let journal = DeleteJournal::from_sources(
            journal_id,
            book_id,
            targets.iter().map(|target| target.source.clone()).collect(),
        )
        .map_err(journal_app_error)?;
        let journal_fault = DeleteJournalFault { fault };
        store
            .write_intent(&journal, &journal_fault)
            .map_err(map_journal_operation_error)?;

        checkpoint(fault, DeleteBoundary::BeforeTrashRootCreate)?;
        let journal_trash_root = store
            .create_journal_trash_root(journal_id)
            .map_err(journal_app_error)?;
        checkpoint(fault, DeleteBoundary::TrashRootCreated)?;
        for (index, (target, entry)) in targets.iter().zip(journal.entries()).enumerate() {
            checkpoint(fault, DeleteBoundary::BeforeStageMove(index))?;
            let destination = store.resolve(entry.trash()).map_err(journal_app_error)?;
            stage_target(
                &self.paths.root,
                target,
                &destination,
                &journal_trash_root,
                index,
                fault,
            )?;
            checkpoint(fault, DeleteBoundary::AfterStageMove(index))?;
        }
        verify_staged_targets(&self.paths.root, &targets, &journal, &store)?;

        let database_fault = DeleteDatabaseFault { fault };
        let deleted = books::complete_delete(&self.pool, book_id, &database_fault)
            .await
            .map_err(map_complete_delete_error)?;
        store
            .write_commit_marker(journal_id, &journal_fault)
            .map_err(map_journal_operation_error)?;

        finalize_staged_targets(
            &self.paths.root,
            &journal,
            &store,
            &journal_trash_root,
            fault,
        )?;
        store
            .remove_journal_files(journal_id, &journal_fault)
            .map_err(map_journal_operation_error)?;
        Ok(outcome(deleted))
    }
}

impl fmt::Debug for DeleteBookService {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("DeleteBookService(<redacted>)")
    }
}

pub fn recover_pending_deletions(pool: &SqlitePool, paths: &AppPaths) -> AppResult<()> {
    tauri::async_runtime::block_on(recover_pending_deletions_async(pool, paths))
}

pub async fn recover_pending_deletions_async(pool: &SqlitePool, paths: &AppPaths) -> AppResult<()> {
    let store = JournalStore::open(paths).map_err(journal_app_error)?;
    store
        .cleanup_stale_temporary_files()
        .map_err(journal_app_error)?;
    let journals = store.list_intents().map_err(journal_app_error)?;
    let Some(total_journal_entries) = journals.iter().try_fold(0_usize, |total, journal| {
        total.checked_add(journal.entries().len())
    }) else {
        return Err(AppError::new(AppErrorCode::RequestConflict));
    };
    if total_journal_entries > MAX_DELETE_JOURNAL_ENTRIES {
        return Err(AppError::new(AppErrorCode::RequestConflict));
    }
    let mut seen_books = HashSet::with_capacity(journals.len());
    if journals
        .iter()
        .any(|journal| !seen_books.insert(journal.book_id()))
    {
        return Err(AppError::new(AppErrorCode::InvalidInput));
    }
    let mut plans = Vec::with_capacity(journals.len());
    for journal in journals {
        plans.push(preflight_recovery_journal(pool, paths, &store, journal).await?);
    }
    validate_recovery_plan_set(&plans)?;
    for plan in &plans {
        apply_recovery_plan(paths, &store, plan)?;
    }
    cleanup_empty_orphan_trash(&store, &HashSet::new())?;
    store
        .cleanup_stale_temporary_files()
        .map_err(journal_app_error)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RecoveryAction {
    KeepSource,
    RestoreTrash,
    DeleteSource,
    DeleteTrash,
    AlreadyFinalized,
}

struct RecoveryEntryPlan {
    source: PathBuf,
    trash: PathBuf,
    kind: TargetKind,
    action: RecoveryAction,
    snapshot: Option<TreeSnapshot>,
}

struct RecoveryPlan {
    journal: DeleteJournal,
    entries: Vec<RecoveryEntryPlan>,
}

async fn preflight_recovery_journal(
    pool: &SqlitePool,
    paths: &AppPaths,
    store: &JournalStore,
    journal: DeleteJournal,
) -> AppResult<RecoveryPlan> {
    let import_status =
        sqlx::query_scalar::<_, String>("SELECT import_status FROM books WHERE id = ?")
            .bind(journal.book_id().to_string())
            .fetch_optional(pool)
            .await
            .map_err(AppError::from)?;
    if import_status
        .as_deref()
        .is_some_and(|status| !matches!(status, "ready" | "failed"))
    {
        return Err(AppError::new(AppErrorCode::RequestConflict));
    }
    let book_exists = import_status.is_some();
    let mut entries = Vec::with_capacity(journal.entries().len());
    for entry in journal.entries() {
        validate_recovery_page_ownership(pool, entry.source(), journal.book_id(), book_exists)
            .await?;
        let source = store.resolve(entry.source()).map_err(journal_app_error)?;
        let trash = store.resolve(entry.trash()).map_err(journal_app_error)?;
        let kind = recovery_target_kind(entry.source(), journal.book_id())?;
        ensure_safe_parent(&paths.root, source.parent().ok_or_else(invalid_input)?)?;
        let trash_parent = trash.parent().ok_or_else(invalid_input)?;
        let trash_parent_exists = path_exists(trash_parent)?;
        if trash_parent_exists {
            ensure_safe_parent(&paths.root, trash_parent)?;
        }
        let source_exists = path_exists(&source)?;
        let trash_exists = path_exists(&trash)?;
        if trash_exists && !trash_parent_exists {
            return Err(invalid_input());
        }
        let (action, snapshot) = match (book_exists, source_exists, trash_exists) {
            (true, true, false) => (
                RecoveryAction::KeepSource,
                Some(capture_tree(&paths.root, &source, kind)?),
            ),
            (true, false, true) => {
                require_same_volume(&trash, source.parent().ok_or_else(invalid_input)?)?;
                (
                    RecoveryAction::RestoreTrash,
                    Some(capture_tree(&paths.root, &trash, kind)?),
                )
            }
            (false, true, false) => (
                RecoveryAction::DeleteSource,
                Some(capture_tree(&paths.root, &source, kind)?),
            ),
            (false, false, true) => (
                RecoveryAction::DeleteTrash,
                Some(capture_tree(&paths.root, &trash, kind)?),
            ),
            (false, false, false) => (RecoveryAction::AlreadyFinalized, None),
            _ => return Err(invalid_input()),
        };
        entries.push(RecoveryEntryPlan {
            source,
            trash,
            kind,
            action,
            snapshot,
        });
    }
    Ok(RecoveryPlan { journal, entries })
}

async fn validate_recovery_page_ownership(
    pool: &SqlitePool,
    source: &RelativePathToken,
    book_id: Uuid,
    book_exists: bool,
) -> AppResult<()> {
    let Some(page_id) = recovery_page_id(source) else {
        return Ok(());
    };
    let owner = sqlx::query_scalar::<_, String>("SELECT book_id FROM index_pages WHERE id = ?")
        .bind(page_id.to_string())
        .fetch_optional(pool)
        .await
        .map_err(AppError::from)?
        .map(|owner| parse_canonical_uuid(&owner).ok_or_else(invalid_input))
        .transpose()?;
    if (book_exists && owner != Some(book_id)) || (!book_exists && owner.is_some()) {
        return Err(invalid_input());
    }
    Ok(())
}

fn validate_recovery_plan_set(plans: &[RecoveryPlan]) -> AppResult<()> {
    let mut paths = HashSet::new();
    let mut identities = HashSet::new();
    let mut total_file_bytes = 0_u64;
    for entry in plans.iter().flat_map(|plan| &plan.entries) {
        let existing_path = match entry.action {
            RecoveryAction::KeepSource | RecoveryAction::DeleteSource => Some(&entry.source),
            RecoveryAction::RestoreTrash | RecoveryAction::DeleteTrash => Some(&entry.trash),
            RecoveryAction::AlreadyFinalized => None,
        };
        if let Some(existing_path) = existing_path {
            if !paths.insert(normalized_path_identity(existing_path)) {
                return Err(invalid_input());
            }
            let snapshot = entry.snapshot.as_ref().ok_or_else(invalid_input)?;
            for tree_entry in &snapshot.entries {
                if identities.len() >= MAX_DELETE_TREE_ENTRIES {
                    return Err(AppError::new(AppErrorCode::RequestConflict));
                }
                if !identities.insert(tree_entry.identity) {
                    return Err(invalid_input());
                }
                if tree_entry.kind == EntryKind::File {
                    total_file_bytes = total_file_bytes
                        .checked_add(tree_entry.len)
                        .filter(|total| *total <= MAX_DELETE_TREE_FILE_BYTES)
                        .ok_or_else(|| AppError::new(AppErrorCode::RequestConflict))?;
                }
            }
        }
    }
    Ok(())
}

fn apply_recovery_plan(
    paths: &AppPaths,
    store: &JournalStore,
    plan: &RecoveryPlan,
) -> AppResult<()> {
    for entry in &plan.entries {
        let snapshot = entry.snapshot.as_ref();
        match entry.action {
            RecoveryAction::KeepSource => {
                if path_exists(&entry.trash)?
                    || capture_tree(&paths.root, &entry.source, entry.kind)?
                        .ne(snapshot.ok_or_else(invalid_input)?)
                {
                    return Err(invalid_input());
                }
            }
            RecoveryAction::RestoreTrash => {
                if path_exists(&entry.source)? || !path_exists(&entry.trash)? {
                    return Err(invalid_input());
                }
                require_same_volume(
                    &entry.trash,
                    entry.source.parent().ok_or_else(invalid_input)?,
                )?;
                if capture_tree(&paths.root, &entry.trash, entry.kind)?
                    .ne(snapshot.ok_or_else(invalid_input)?)
                {
                    return Err(invalid_input());
                }
                rename_same_volume(&entry.trash, &entry.source)?;
                sync_owned_directory(entry.source.parent().ok_or_else(invalid_input)?)
                    .map_err(journal_app_error)?;
                sync_owned_directory(entry.trash.parent().ok_or_else(invalid_input)?)
                    .map_err(journal_app_error)?;
                if capture_tree(&paths.root, &entry.source, entry.kind)?
                    .ne(snapshot.ok_or_else(invalid_input)?)
                {
                    return Err(invalid_input());
                }
            }
            RecoveryAction::DeleteSource => {
                if path_exists(&entry.trash)? {
                    return Err(invalid_input());
                }
                remove_tree_checked(
                    &paths.root,
                    &entry.source,
                    entry.kind,
                    snapshot.ok_or_else(invalid_input)?,
                )?;
                sync_owned_directory(entry.source.parent().ok_or_else(invalid_input)?)
                    .map_err(journal_app_error)?;
            }
            RecoveryAction::DeleteTrash => {
                if path_exists(&entry.source)? {
                    return Err(invalid_input());
                }
                remove_tree_checked(
                    &paths.root,
                    &entry.trash,
                    entry.kind,
                    snapshot.ok_or_else(invalid_input)?,
                )?;
                sync_owned_directory(entry.trash.parent().ok_or_else(invalid_input)?)
                    .map_err(journal_app_error)?;
            }
            RecoveryAction::AlreadyFinalized => {
                if path_exists(&entry.source)? || path_exists(&entry.trash)? {
                    return Err(invalid_input());
                }
            }
        }
    }
    let trash_root = store.journal_trash_root(plan.journal.journal_id());
    remove_empty_directory(&paths.root, &trash_root, store.trash_directory())?;
    store
        .remove_journal_files(plan.journal.journal_id(), &NoJournalFault)
        .map_err(map_journal_operation_app_error)
}

fn require_terminal_plan(plan: &BookDeletePlan, policy: DeletePolicy) -> AppResult<()> {
    if plan.book_id.is_nil()
        || plan.has_nonterminal_indexing
        || !matches!(
            plan.import_status,
            ImportStatus::Ready | ImportStatus::Failed
        )
        || (policy == DeletePolicy::FailedOnly && plan.import_status != ImportStatus::Failed)
    {
        return Err(AppError::new(AppErrorCode::RequestConflict));
    }
    validate_stored_path(plan)
}

fn validate_stored_path(plan: &BookDeletePlan) -> AppResult<()> {
    let expected = format!(
        "books/{}/original.{}",
        plan.book_id,
        format_extension(&plan.format)
    );
    if plan
        .stored_path
        .as_deref()
        .is_some_and(|stored_path| stored_path != expected)
    {
        return Err(invalid_input());
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TargetKind {
    BookExact(&'static str),
    BookAny,
    PageScratch,
}

#[derive(Clone)]
struct ValidatedTarget {
    source: RelativePathToken,
    path: PathBuf,
    parent: PathBuf,
    kind: TargetKind,
    snapshot: TreeSnapshot,
}

impl fmt::Debug for ValidatedTarget {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ValidatedTarget")
            .field("kind", &self.kind)
            .field("entry_count", &self.snapshot.entries.len())
            .field("paths", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum EntryKind {
    File,
    Directory,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct FileIdentity {
    volume: u64,
    index: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TreeEntry {
    relative: String,
    kind: EntryKind,
    identity: FileIdentity,
    len: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TreeSnapshot {
    entries: Vec<TreeEntry>,
}

fn collect_delete_targets(
    paths: &AppPaths,
    store: &JournalStore,
    plan: &BookDeletePlan,
) -> AppResult<Vec<ValidatedTarget>> {
    validate_stored_path(plan)?;
    let mut targets = Vec::new();
    let book_token =
        RelativePathToken::new(format!("books/{}", plan.book_id)).map_err(journal_app_error)?;
    let book_path = store.resolve(&book_token).map_err(journal_app_error)?;
    if path_exists(&book_path)? {
        let kind = TargetKind::BookExact(format_extension(&plan.format));
        let snapshot = capture_tree(&paths.root, &book_path, kind)?;
        targets.push(ValidatedTarget {
            source: book_token,
            parent: paths.books.clone(),
            path: book_path,
            kind,
            snapshot,
        });
    }

    let scratch_root = paths.indexing_scratch();
    if path_exists(&scratch_root)? {
        ensure_exact_owned_directory(&paths.root, &paths.cache, "cache")?;
        ensure_exact_owned_directory(&paths.root, &scratch_root, "indexing-pages")?;
        for page_id in &plan.page_ids {
            let token = RelativePathToken::new(format!("cache/indexing-pages/{page_id}"))
                .map_err(journal_app_error)?;
            let path = store.resolve(&token).map_err(journal_app_error)?;
            if !path_exists(&path)? {
                continue;
            }
            let snapshot = capture_tree(&paths.root, &path, TargetKind::PageScratch)?;
            targets.push(ValidatedTarget {
                source: token,
                parent: scratch_root.clone(),
                path,
                kind: TargetKind::PageScratch,
                snapshot,
            });
        }
    }

    let mut identities = HashSet::new();
    let mut path_identities = HashSet::new();
    for target in &targets {
        let normalized = normalized_path_identity(&target.path);
        if !path_identities.insert(normalized)
            || target
                .snapshot
                .entries
                .iter()
                .any(|entry| !identities.insert(entry.identity))
        {
            return Err(invalid_input());
        }
    }
    Ok(targets)
}

fn stage_target(
    app_root: &Path,
    target: &ValidatedTarget,
    destination: &Path,
    journal_trash_root: &Path,
    index: usize,
    fault: &dyn DeleteFaultInjector,
) -> Result<(), DeleteBookError> {
    if capture_tree(app_root, &target.path, target.kind)? != target.snapshot {
        return Err(invalid_input().into());
    }
    ensure_safe_parent(app_root, &target.parent)?;
    ensure_safe_parent(app_root, journal_trash_root)?;
    if destination.parent() != Some(journal_trash_root) || path_exists(destination)? {
        return Err(invalid_input().into());
    }
    require_same_volume(&target.path, journal_trash_root)?;
    rename_same_volume(&target.path, destination)?;
    checkpoint(fault, DeleteBoundary::StageMoved(index))?;
    sync_owned_directory(&target.parent).map_err(journal_app_error)?;
    checkpoint(fault, DeleteBoundary::StageSourceParentSynced(index))?;
    sync_owned_directory(journal_trash_root).map_err(journal_app_error)?;
    checkpoint(fault, DeleteBoundary::StageTrashParentSynced(index))?;
    if path_exists(&target.path)?
        || capture_tree(app_root, destination, target.kind)? != target.snapshot
    {
        let _ = rename_owned_atomic(destination, &target.path);
        let _ = sync_owned_directory(&target.parent);
        let _ = sync_owned_directory(journal_trash_root);
        return Err(invalid_input().into());
    }
    checkpoint(fault, DeleteBoundary::StageVerified(index))?;
    Ok(())
}

fn verify_staged_targets(
    app_root: &Path,
    targets: &[ValidatedTarget],
    journal: &DeleteJournal,
    store: &JournalStore,
) -> AppResult<()> {
    for (target, entry) in targets.iter().zip(journal.entries()) {
        let trash = store.resolve(entry.trash()).map_err(journal_app_error)?;
        if path_exists(&target.path)?
            || capture_tree(app_root, &trash, target.kind)? != target.snapshot
        {
            return Err(invalid_input());
        }
    }
    Ok(())
}

fn finalize_staged_targets(
    app_root: &Path,
    journal: &DeleteJournal,
    store: &JournalStore,
    journal_trash_root: &Path,
    fault: &dyn DeleteFaultInjector,
) -> Result<(), DeleteBookError> {
    for (index, entry) in journal.entries().iter().enumerate() {
        checkpoint(fault, DeleteBoundary::BeforeFinalizeEntry(index))?;
        let trash = store.resolve(entry.trash()).map_err(journal_app_error)?;
        if path_exists(&trash)? {
            let kind = recovery_target_kind(entry.source(), journal.book_id())?;
            let snapshot = capture_tree(app_root, &trash, kind)?;
            remove_tree_checked(app_root, &trash, kind, &snapshot)?;
            checkpoint(fault, DeleteBoundary::FinalizeEntryRemoved(index))?;
            sync_owned_directory(trash.parent().ok_or_else(invalid_input)?)
                .map_err(journal_app_error)?;
            checkpoint(fault, DeleteBoundary::FinalizeEntryParentSynced(index))?;
        }
        checkpoint(fault, DeleteBoundary::AfterFinalizeEntry(index))?;
    }
    checkpoint(fault, DeleteBoundary::BeforeTrashRootRemove)?;
    remove_empty_directory(app_root, journal_trash_root, store.trash_directory())?;
    checkpoint(fault, DeleteBoundary::TrashRootRemoved)
}

fn capture_tree(app_root: &Path, target: &Path, kind: TargetKind) -> AppResult<TreeSnapshot> {
    let canonical_root = fs::canonicalize(app_root).map_err(|_| local_io())?;
    if canonical_root != app_root {
        return Err(invalid_input());
    }
    let mut entries = Vec::new();
    let mut relative_identities = HashSet::new();
    let mut total_file_bytes = 0_u64;
    capture_tree_node(
        &canonical_root,
        target,
        target,
        kind,
        &mut entries,
        &mut relative_identities,
        &mut total_file_bytes,
    )?;
    entries.sort_unstable_by(|left, right| left.relative.cmp(&right.relative));
    let mut file_identities = HashSet::with_capacity(entries.len());
    if entries
        .iter()
        .any(|entry| !file_identities.insert(entry.identity))
    {
        return Err(invalid_input());
    }
    Ok(TreeSnapshot { entries })
}

fn capture_tree_node(
    app_root: &Path,
    target_root: &Path,
    path: &Path,
    target_kind: TargetKind,
    entries: &mut Vec<TreeEntry>,
    relative_identities: &mut HashSet<String>,
    total_file_bytes: &mut u64,
) -> AppResult<()> {
    if entries.len() >= MAX_DELETE_TREE_ENTRIES {
        return Err(AppError::new(AppErrorCode::RequestConflict));
    }
    let metadata = fs::symlink_metadata(path).map_err(|_| local_io())?;
    if metadata_is_reparse(&metadata) || (!metadata.is_file() && !metadata.is_dir()) {
        return Err(invalid_input());
    }
    let canonical = fs::canonicalize(path).map_err(|_| local_io())?;
    if canonical != path || !canonical.starts_with(app_root) || canonical == app_root {
        return Err(invalid_input());
    }
    let relative = path
        .strip_prefix(target_root)
        .map_err(|_| invalid_input())?;
    let relative = relative_path_text(relative)?;
    if !relative.is_empty() && relative.split('/').count() > MAX_DELETE_TREE_DEPTH {
        return Err(AppError::new(AppErrorCode::RequestConflict));
    }
    let entry_kind = if metadata.is_dir() {
        EntryKind::Directory
    } else {
        EntryKind::File
    };
    validate_target_shape(target_kind, &relative, entry_kind)?;
    if !relative_identities.insert(relative.to_lowercase()) {
        return Err(invalid_input());
    }
    if metadata.is_file() {
        *total_file_bytes = total_file_bytes
            .checked_add(metadata.len())
            .filter(|total| *total <= MAX_DELETE_TREE_FILE_BYTES)
            .ok_or_else(|| AppError::new(AppErrorCode::RequestConflict))?;
    }
    entries.push(TreeEntry {
        relative,
        kind: entry_kind,
        identity: metadata_identity(path, &metadata)?,
        len: metadata.len(),
    });
    if metadata.is_dir() {
        let mut children = fs::read_dir(path)
            .map_err(|_| local_io())?
            .map(|entry| entry.map(|entry| entry.path()).map_err(|_| local_io()))
            .collect::<AppResult<Vec<_>>>()?;
        children.sort_unstable_by(|left, right| {
            normalized_path_identity(left).cmp(&normalized_path_identity(right))
        });
        for child in children {
            capture_tree_node(
                app_root,
                target_root,
                &child,
                target_kind,
                entries,
                relative_identities,
                total_file_bytes,
            )?;
        }
        let after = fs::symlink_metadata(path).map_err(|_| local_io())?;
        if metadata_is_reparse(&after)
            || metadata_identity(path, &metadata)? != metadata_identity(path, &after)?
        {
            return Err(invalid_input());
        }
    }
    Ok(())
}

fn validate_target_shape(
    target_kind: TargetKind,
    relative: &str,
    entry_kind: EntryKind,
) -> AppResult<()> {
    if relative.is_empty() {
        return if entry_kind == EntryKind::Directory {
            Ok(())
        } else {
            Err(invalid_input())
        };
    }
    let components = relative.split('/').collect::<Vec<_>>();
    if components
        .iter()
        .any(|component| !safe_fs_component(component))
    {
        return Err(invalid_input());
    }
    match target_kind {
        TargetKind::BookExact(extension) => {
            validate_book_shape(&components, entry_kind, Some(extension))
        }
        TargetKind::BookAny => validate_book_shape(&components, entry_kind, None),
        TargetKind::PageScratch => {
            if components.len() != 1 || entry_kind != EntryKind::File {
                return Err(invalid_input());
            }
            let Some((attempt, suffix)) = components[0].rsplit_once('.') else {
                return Err(invalid_input());
            };
            if !matches!(suffix, "render" | "tmp") || parse_canonical_uuid(attempt).is_none() {
                return Err(invalid_input());
            }
            Ok(())
        }
    }
}

fn validate_book_shape(
    components: &[&str],
    entry_kind: EntryKind,
    exact_extension: Option<&str>,
) -> AppResult<()> {
    if components[0] == "derived" {
        if components.len() == 1 && entry_kind != EntryKind::Directory {
            return Err(invalid_input());
        }
        return Ok(());
    }
    if components.len() != 1 || entry_kind != EntryKind::File {
        return Err(invalid_input());
    }
    let valid = match exact_extension {
        Some(extension) => {
            components[0] == format!("original.{extension}")
                || components[0] == format!("original.{extension}.partial")
        }
        None => ["pdf", "epub", "docx"].into_iter().any(|extension| {
            components[0] == format!("original.{extension}")
                || components[0] == format!("original.{extension}.partial")
        }),
    };
    if valid { Ok(()) } else { Err(invalid_input()) }
}

fn remove_tree_checked(
    app_root: &Path,
    path: &Path,
    kind: TargetKind,
    expected: &TreeSnapshot,
) -> AppResult<()> {
    if capture_tree(app_root, path, kind)? != *expected {
        return Err(invalid_input());
    }
    remove_tree_node(app_root, path)
}

fn remove_tree_node(app_root: &Path, path: &Path) -> AppResult<()> {
    let before = fs::symlink_metadata(path).map_err(|_| local_io())?;
    if metadata_is_reparse(&before) || (!before.is_file() && !before.is_dir()) {
        return Err(invalid_input());
    }
    let canonical = fs::canonicalize(path).map_err(|_| local_io())?;
    if canonical != path || !canonical.starts_with(app_root) || canonical == app_root {
        return Err(invalid_input());
    }
    if before.is_dir() {
        let children = fs::read_dir(path)
            .map_err(|_| local_io())?
            .map(|entry| entry.map(|entry| entry.path()).map_err(|_| local_io()))
            .collect::<AppResult<Vec<_>>>()?;
        for child in children {
            remove_tree_node(app_root, &child)?;
        }
        let after = fs::symlink_metadata(path).map_err(|_| local_io())?;
        if metadata_is_reparse(&after)
            || metadata_identity(path, &before)? != metadata_identity(path, &after)?
        {
            return Err(invalid_input());
        }
        fs::remove_dir(path).map_err(|_| local_io())?;
    } else {
        fs::remove_file(path).map_err(|_| local_io())?;
    }
    Ok(())
}

fn recovery_target_kind(token: &RelativePathToken, book_id: Uuid) -> AppResult<TargetKind> {
    if token.as_str() == format!("books/{book_id}") {
        return Ok(TargetKind::BookAny);
    }
    if recovery_page_id(token).is_some() {
        return Ok(TargetKind::PageScratch);
    }
    Err(invalid_input())
}

fn recovery_page_id(token: &RelativePathToken) -> Option<Uuid> {
    token
        .as_str()
        .strip_prefix("cache/indexing-pages/")
        .and_then(parse_canonical_uuid)
}

fn cleanup_empty_orphan_trash(
    store: &JournalStore,
    active_journal_ids: &HashSet<Uuid>,
) -> AppResult<()> {
    for entry in fs::read_dir(store.trash_directory()).map_err(|_| local_io())? {
        let entry = entry.map_err(|_| local_io())?;
        let name = entry
            .file_name()
            .to_str()
            .and_then(parse_canonical_uuid)
            .ok_or_else(invalid_input)?;
        if active_journal_ids.contains(&name) {
            continue;
        }
        remove_empty_directory(
            store.root_directory(),
            &entry.path(),
            store.trash_directory(),
        )?;
    }
    Ok(())
}

fn remove_empty_directory(app_root: &Path, path: &Path, expected_parent: &Path) -> AppResult<()> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => return Err(local_io()),
    };
    if !metadata.is_dir() || metadata_is_reparse(&metadata) {
        return Err(invalid_input());
    }
    let canonical = fs::canonicalize(path).map_err(|_| local_io())?;
    if canonical != path
        || canonical.parent() != Some(expected_parent)
        || !canonical.starts_with(app_root)
        || fs::read_dir(path).map_err(|_| local_io())?.next().is_some()
    {
        return Err(invalid_input());
    }
    fs::remove_dir(path).map_err(|_| local_io())?;
    sync_owned_directory(expected_parent).map_err(journal_app_error)
}

fn ensure_exact_owned_directory(
    app_root: &Path,
    path: &Path,
    expected_name: &str,
) -> AppResult<()> {
    let metadata = fs::symlink_metadata(path).map_err(|_| local_io())?;
    let canonical = fs::canonicalize(path).map_err(|_| local_io())?;
    if !metadata.is_dir()
        || metadata_is_reparse(&metadata)
        || canonical != path
        || !canonical.starts_with(app_root)
        || canonical.file_name().and_then(|name| name.to_str()) != Some(expected_name)
    {
        return Err(invalid_input());
    }
    Ok(())
}

fn ensure_safe_parent(app_root: &Path, parent: &Path) -> AppResult<()> {
    let metadata = fs::symlink_metadata(parent).map_err(|_| local_io())?;
    let canonical = fs::canonicalize(parent).map_err(|_| local_io())?;
    if !metadata.is_dir()
        || metadata_is_reparse(&metadata)
        || canonical != parent
        || !canonical.starts_with(app_root)
        || canonical == app_root
    {
        return Err(invalid_input());
    }
    Ok(())
}

fn require_same_volume(source: &Path, destination_parent: &Path) -> AppResult<()> {
    let source_metadata = fs::symlink_metadata(source).map_err(|_| local_io())?;
    let destination_metadata = fs::symlink_metadata(destination_parent).map_err(|_| local_io())?;
    if filesystem_volume(source, &source_metadata)?
        != filesystem_volume(destination_parent, &destination_metadata)?
    {
        return Err(invalid_input());
    }
    Ok(())
}

fn rename_same_volume(source: &Path, destination: &Path) -> AppResult<()> {
    if path_exists(destination)? {
        return Err(invalid_input());
    }
    rename_owned_atomic(source, destination).map_err(journal_app_error)
}

fn path_exists(path: &Path) -> AppResult<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(_) => Err(local_io()),
    }
}

fn relative_path_text(path: &Path) -> AppResult<String> {
    let text = path.to_str().ok_or_else(invalid_input)?.replace('\\', "/");
    if text.starts_with('/') || text.ends_with('/') || text.contains("//") {
        return Err(invalid_input());
    }
    Ok(text)
}

fn safe_fs_component(component: &str) -> bool {
    !component.is_empty()
        && component != "."
        && component != ".."
        && !component.ends_with('.')
        && !component.ends_with(' ')
        && !component.contains(['\\', '/', ':', '\0'])
        && !component
            .chars()
            .any(|character| character.is_ascii_control() || "<>\"|?*".contains(character))
        && !is_windows_device_name(component)
}

fn is_windows_device_name(component: &str) -> bool {
    let stem = component
        .split_once('.')
        .map_or(component, |(stem, _)| stem)
        .to_ascii_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || stem
            .strip_prefix("COM")
            .or_else(|| stem.strip_prefix("LPT"))
            .is_some_and(|suffix| suffix.len() == 1 && matches!(suffix.as_bytes()[0], b'1'..=b'9'))
}

fn parse_canonical_uuid(value: &str) -> Option<Uuid> {
    let id = Uuid::parse_str(value).ok()?;
    (id.to_string() == value).then_some(id)
}

fn normalized_path_identity(path: &Path) -> String {
    #[cfg(windows)]
    {
        path.as_os_str().to_string_lossy().to_lowercase()
    }
    #[cfg(not(windows))]
    {
        path.as_os_str().to_string_lossy().into_owned()
    }
}

#[cfg(unix)]
fn metadata_identity(_path: &Path, metadata: &fs::Metadata) -> AppResult<FileIdentity> {
    use std::os::unix::fs::MetadataExt;

    Ok(FileIdentity {
        volume: metadata.dev(),
        index: metadata.ino(),
    })
}

#[cfg(windows)]
fn metadata_identity(path: &Path, _metadata: &fs::Metadata) -> AppResult<FileIdentity> {
    windows_path_identity(path)
}

#[cfg(not(any(unix, windows)))]
fn metadata_identity(_path: &Path, _metadata: &fs::Metadata) -> AppResult<FileIdentity> {
    Err(invalid_input())
}

#[cfg(unix)]
fn filesystem_volume(_path: &Path, metadata: &fs::Metadata) -> AppResult<u64> {
    use std::os::unix::fs::MetadataExt;

    Ok(metadata.dev())
}

#[cfg(windows)]
fn filesystem_volume(path: &Path, _metadata: &fs::Metadata) -> AppResult<u64> {
    Ok(windows_path_identity(path)?.volume)
}

#[cfg(not(any(unix, windows)))]
fn filesystem_volume(_path: &Path, _metadata: &fs::Metadata) -> AppResult<u64> {
    Err(invalid_input())
}

#[cfg(windows)]
#[repr(C)]
struct WindowsFileTime {
    low: u32,
    high: u32,
}

#[cfg(windows)]
#[repr(C)]
struct WindowsByHandleFileInformation {
    attributes: u32,
    creation_time: WindowsFileTime,
    last_access_time: WindowsFileTime,
    last_write_time: WindowsFileTime,
    volume_serial_number: u32,
    file_size_high: u32,
    file_size_low: u32,
    number_of_links: u32,
    file_index_high: u32,
    file_index_low: u32,
}

#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    #[link_name = "CreateFileW"]
    fn create_file_w(
        file_name: *const u16,
        desired_access: u32,
        share_mode: u32,
        security_attributes: *mut std::ffi::c_void,
        creation_disposition: u32,
        flags_and_attributes: u32,
        template_file: *mut std::ffi::c_void,
    ) -> *mut std::ffi::c_void;

    #[link_name = "GetFileInformationByHandle"]
    fn get_file_information_by_handle(
        file: *mut std::ffi::c_void,
        information: *mut WindowsByHandleFileInformation,
    ) -> i32;

    #[link_name = "CloseHandle"]
    fn close_handle(handle: *mut std::ffi::c_void) -> i32;
}

#[cfg(windows)]
fn windows_path_identity(path: &Path) -> AppResult<FileIdentity> {
    use std::os::windows::ffi::OsStrExt;

    const FILE_READ_ATTRIBUTES: u32 = 0x0080;
    const FILE_SHARE_READ: u32 = 0x0000_0001;
    const FILE_SHARE_WRITE: u32 = 0x0000_0002;
    const FILE_SHARE_DELETE: u32 = 0x0000_0004;
    const OPEN_EXISTING: u32 = 3;
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;

    let mut wide = path.as_os_str().encode_wide().collect::<Vec<_>>();
    wide.push(0);
    // SAFETY: `wide` is a live, NUL-terminated UTF-16 buffer; all optional
    // pointers are null and the returned handle is checked before use.
    let handle = unsafe {
        create_file_w(
            wide.as_ptr(),
            FILE_READ_ATTRIBUTES,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null_mut(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            std::ptr::null_mut(),
        )
    };
    if handle as isize == -1 {
        return Err(local_io());
    }
    // SAFETY: the all-integer C representation admits an all-zero value.
    let mut information: WindowsByHandleFileInformation = unsafe { std::mem::zeroed() };
    // SAFETY: `handle` is valid and `information` points to writable storage
    // with the exact Windows BY_HANDLE_FILE_INFORMATION layout.
    let succeeded = unsafe { get_file_information_by_handle(handle, &mut information) } != 0;
    // SAFETY: `handle` was returned by CreateFileW and is closed exactly once.
    let _ = unsafe { close_handle(handle) };
    if !succeeded {
        return Err(local_io());
    }
    Ok(FileIdentity {
        volume: u64::from(information.volume_serial_number),
        index: (u64::from(information.file_index_high) << 32)
            | u64::from(information.file_index_low),
    })
}

struct DeleteJournalFault<'a> {
    fault: &'a dyn DeleteFaultInjector,
}

impl JournalFaultInjector for DeleteJournalFault<'_> {
    fn checkpoint(&self, boundary: JournalBoundary) -> Result<(), JournalInjectedCrash> {
        self.fault
            .checkpoint(DeleteBoundary::Journal(boundary))
            .map_err(|_| JournalInjectedCrash::at(boundary))
    }
}

struct DeleteDatabaseFault<'a> {
    fault: &'a dyn DeleteFaultInjector,
}

impl CompleteDeleteFaultInjector for DeleteDatabaseFault<'_> {
    fn checkpoint(&self, step: CompleteDeleteStep) -> Result<(), CompleteDeleteInjectedCrash> {
        self.fault
            .checkpoint(DeleteBoundary::Database(step))
            .map_err(|_| CompleteDeleteInjectedCrash::at(step))
    }
}

fn checkpoint(
    fault: &dyn DeleteFaultInjector,
    boundary: DeleteBoundary,
) -> Result<(), DeleteBookError> {
    fault
        .checkpoint(boundary)
        .map_err(|_| DeleteBookError::Injected(boundary))
}

fn map_journal_operation_error(error: JournalOperationError) -> DeleteBookError {
    match error {
        JournalOperationError::Journal(error) => DeleteBookError::App(journal_app_error(error)),
        JournalOperationError::Injected(crash) => {
            DeleteBookError::Injected(DeleteBoundary::Journal(crash.boundary()))
        }
    }
}

fn map_journal_operation_app_error(error: JournalOperationError) -> AppError {
    match error {
        JournalOperationError::Journal(error) => journal_app_error(error),
        JournalOperationError::Injected(_) => AppError::new(AppErrorCode::LocalIoError),
    }
}

fn map_complete_delete_error(error: CompleteDeleteError) -> DeleteBookError {
    match error {
        CompleteDeleteError::App(error) => DeleteBookError::App(error),
        CompleteDeleteError::Injected(crash) => {
            DeleteBookError::Injected(DeleteBoundary::Database(crash.step()))
        }
    }
}

fn outcome(result: CompleteDeleteResult) -> DeleteBookOutcome {
    DeleteBookOutcome {
        detached_remote_resource_ids: result.detached_remote_resource_ids().to_vec(),
    }
}

fn journal_app_error(error: JournalError) -> AppError {
    error.into_app_error()
}

fn format_extension(format: &BookFormat) -> &'static str {
    match format {
        BookFormat::Pdf => "pdf",
        BookFormat::Epub => "epub",
        BookFormat::Docx => "docx",
    }
}

fn invalid_input() -> AppError {
    AppError::new(AppErrorCode::InvalidInput)
}

fn local_io() -> AppError {
    AppError::new(AppErrorCode::LocalIoError)
}

#[cfg(test)]
#[path = "delete_book_test.rs"]
mod tests;
