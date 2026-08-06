use std::{
    collections::{HashMap, HashSet},
    ffi::OsStr,
    fmt, fs,
    io::{Read, Seek, SeekFrom, Write},
    path::{Component, Path, PathBuf},
    sync::Arc,
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{Connection, Row, SqliteConnection, SqlitePool, sqlite::SqliteOwnedBuf};
use uuid::Uuid;

use crate::{
    app_state::AppPaths,
    domain::{BackupSummaryDto, MaintenanceErrorCode, MaintenanceErrorDto},
    maintenance::{
        gate::{GateAcquireError, MaintenanceGate},
        journal::metadata_is_reparse,
        storage::{APP_DATA_DIRECTORY_NAME, StorageLayout},
    },
};

pub const BACKUP_FORMAT_VERSION: u32 = 1;

const HEADER_MAGIC: &[u8; 8] = b"TLBACKUP";
const FOOTER_MAGIC: &[u8; 8] = b"TLBEND01";
const FORMAT_IDENTIFIER: &str = "textbooklens-local-backup";
const HEADER_BYTES: u64 = 16;
const FOOTER_BYTES: u64 = 48;
const COPY_BUFFER_BYTES: usize = 64 * 1024;
const MAX_ARCHIVE_BYTES: u64 = 1_099_511_627_776;
const MAX_DATABASE_BYTES: u64 = 8_589_934_592;
const MAX_ENTRY_BYTES: u64 = 274_877_906_944;
const MAX_ENTRY_COUNT: usize = 100_000;
const MAX_MANIFEST_BYTES: u64 = 33_554_432;
const MAX_RELATIVE_PATH_BYTES: usize = 1_024;
const MAX_DESTINATION_PATH_BYTES: usize = 32_768;
const MAX_PATH_DEPTH: usize = 32;
const TEMP_FILE_PREFIX: &str = ".textbooklens-backup-";

const REMOTE_RESOURCE_DELETE_TRIGGER: &str = r#"
CREATE TRIGGER provider_remote_resources_delete_only_resolved
BEFORE DELETE ON provider_remote_resources
WHEN old.cleanup_status NOT IN ('succeeded', 'safely_disposed')
BEGIN
  SELECT RAISE(ABORT, 'unresolved remote resource must be retained');
END
"#;

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct BackupArchiveError {
    pub code: MaintenanceErrorCode,
}

impl BackupArchiveError {
    const fn new(code: MaintenanceErrorCode) -> Self {
        Self { code }
    }
}

impl fmt::Debug for BackupArchiveError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BackupArchiveError")
            .field("code", &self.code)
            .finish()
    }
}

impl fmt::Display for BackupArchiveError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:?}", self.code)
    }
}

impl std::error::Error for BackupArchiveError {}

#[derive(PartialEq, Eq)]
pub enum BackupError {
    Gate(GateAcquireError),
    Archive(BackupArchiveError),
}

impl fmt::Debug for BackupError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Gate(error) => formatter.debug_tuple("Gate").field(error).finish(),
            Self::Archive(error) => formatter.debug_tuple("Archive").field(error).finish(),
        }
    }
}

impl fmt::Display for BackupError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Gate(_) => formatter.write_str("backup maintenance gate unavailable"),
            Self::Archive(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for BackupError {}

impl From<GateAcquireError> for BackupError {
    fn from(error: GateAcquireError) -> Self {
        Self::Gate(error)
    }
}

impl From<BackupArchiveError> for BackupError {
    fn from(error: BackupArchiveError) -> Self {
        Self::Archive(error)
    }
}

impl From<BackupError> for MaintenanceErrorDto {
    fn from(error: BackupError) -> Self {
        match error {
            BackupError::Gate(error) => Self::from(error),
            BackupError::Archive(error) => Self::new(error.code),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BackupBoundary {
    DatabaseSnapshotReady,
    SourcesValidated,
    HeaderWritten,
    EntriesWritten,
    ArchiveSynced,
    ArchiveVerified,
    BeforePublish,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct BackupInjectedFailure;

trait BackupFaultInjector: Send + Sync {
    fn checkpoint(&self, boundary: BackupBoundary) -> Result<(), BackupInjectedFailure>;
}

#[derive(Clone, Copy, Debug, Default)]
struct NoBackupFault;

impl BackupFaultInjector for NoBackupFault {
    fn checkpoint(&self, _boundary: BackupBoundary) -> Result<(), BackupInjectedFailure> {
        Ok(())
    }
}

#[derive(Clone)]
pub struct BackupService {
    pool: SqlitePool,
    paths: AppPaths,
    gate: MaintenanceGate,
    fault: Arc<dyn BackupFaultInjector>,
}

impl BackupService {
    pub fn new(pool: SqlitePool, paths: AppPaths, gate: MaintenanceGate) -> Self {
        Self {
            pool,
            paths,
            gate,
            fault: Arc::new(NoBackupFault),
        }
    }

    #[cfg(test)]
    fn with_fault_injector(
        pool: SqlitePool,
        paths: AppPaths,
        gate: MaintenanceGate,
        fault: Arc<dyn BackupFaultInjector>,
    ) -> Self {
        Self {
            pool,
            paths,
            gate,
            fault,
        }
    }

    pub async fn create_backup(
        &self,
        destination: PathBuf,
    ) -> Result<BackupSummaryDto, BackupError> {
        let permit = self.gate.try_acquire_maintenance()?;
        let layout = ValidatedAppLayout::from_paths(&self.paths)?;
        let destination = ValidatedDestination::new(&destination, &layout)?;
        let database = sanitized_database_snapshot(&self.pool).await?;
        checkpoint(self.fault.as_ref(), BackupBoundary::DatabaseSnapshotReady)?;

        let fault = self.fault.clone();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            create_archive_file(&layout, &destination, database, fault.as_ref())
        })
        .await
        .map_err(|_| write_failed())?
        .map_err(BackupError::from)
    }
}

impl fmt::Debug for BackupService {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("BackupService(<redacted>)")
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct FileIdentity {
    volume: u64,
    index: u64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct StableEntry {
    identity: FileIdentity,
    len: u64,
    modified: std::time::SystemTime,
    links: u64,
}

struct ValidatedAppLayout {
    root: PathBuf,
    books: PathBuf,
    root_entry: StableEntry,
    books_entry: StableEntry,
}

impl ValidatedAppLayout {
    fn from_paths(paths: &AppPaths) -> Result<Self, BackupArchiveError> {
        StorageLayout::from_app_paths(paths).map_err(|_| source_unsafe())?;
        validate_canonical_local_path(&paths.root).map_err(|_| source_unsafe())?;
        ensure_no_reparse_ancestors(&paths.root).map_err(|_| source_unsafe())?;
        let canonical_root = fs::canonicalize(&paths.root).map_err(|_| source_unsafe())?;
        if canonical_root != paths.root
            || canonical_root.file_name() != Some(OsStr::new(APP_DATA_DIRECTORY_NAME))
        {
            return Err(source_unsafe());
        }

        let root_entry = stable_path_entry(&paths.root, EntryKind::Directory)?;
        let books_entry = stable_path_entry(&paths.books, EntryKind::Directory)?;
        let cache_entry = stable_path_entry(&paths.cache, EntryKind::Directory)?;
        let logs_entry = stable_path_entry(&paths.logs, EntryKind::Directory)?;
        if paths.books != paths.root.join("books")
            || paths.cache != paths.root.join("cache")
            || paths.logs != paths.root.join("logs")
            || paths.database != paths.root.join("library.sqlite3")
            || books_entry.identity.volume != root_entry.identity.volume
            || cache_entry.identity.volume != root_entry.identity.volume
            || logs_entry.identity.volume != root_entry.identity.volume
            || books_entry.identity == root_entry.identity
            || cache_entry.identity == root_entry.identity
            || logs_entry.identity == root_entry.identity
            || books_entry.identity == cache_entry.identity
            || books_entry.identity == logs_entry.identity
            || cache_entry.identity == logs_entry.identity
        {
            return Err(source_unsafe());
        }
        if path_exists(&paths.database)? {
            let database_entry = stable_path_entry(&paths.database, EntryKind::File)?;
            if database_entry.identity.volume != root_entry.identity.volume
                || database_entry.links != 1
            {
                return Err(source_unsafe());
            }
        }
        Ok(Self {
            root: paths.root.clone(),
            books: paths.books.clone(),
            root_entry,
            books_entry,
        })
    }

    fn revalidate(&self) -> Result<(), BackupArchiveError> {
        require_stable_path(&self.root, EntryKind::Directory, self.root_entry)?;
        require_stable_path(&self.books, EntryKind::Directory, self.books_entry)?;
        if self.books.parent() != Some(self.root.as_path()) {
            return Err(source_unsafe());
        }
        Ok(())
    }
}

struct ValidatedDestination {
    parent: PathBuf,
    target: PathBuf,
    parent_entry: StableEntry,
}

impl ValidatedDestination {
    fn new(path: &Path, layout: &ValidatedAppLayout) -> Result<Self, BackupArchiveError> {
        validate_local_absolute_path(path).map_err(|_| destination_invalid())?;
        let file_name = path
            .file_name()
            .and_then(OsStr::to_str)
            .filter(|name| safe_component(name))
            .ok_or_else(destination_invalid)?;
        if Path::new(file_name)
            .extension()
            .and_then(OsStr::to_str)
            .is_none_or(|extension| !extension.eq_ignore_ascii_case("tlbackup"))
        {
            return Err(destination_invalid());
        }
        let requested_parent = path.parent().ok_or_else(destination_invalid)?;
        ensure_no_reparse_ancestors(requested_parent).map_err(|_| destination_invalid())?;
        let parent = fs::canonicalize(requested_parent).map_err(|_| destination_invalid())?;
        let parent_entry =
            stable_path_entry(&parent, EntryKind::Directory).map_err(|_| destination_invalid())?;
        let target = parent.join(file_name);
        if path_is_within(&target, &layout.root) {
            return Err(destination_invalid());
        }
        if path_exists(&target)? {
            return Err(destination_exists());
        }
        Ok(Self {
            parent,
            target,
            parent_entry,
        })
    }

    fn require_missing_and_stable(&self) -> Result<(), BackupArchiveError> {
        let current = stable_path_entry(&self.parent, EntryKind::Directory)
            .map_err(|_| destination_invalid())?;
        if current.identity != self.parent_entry.identity
            || current.links != self.parent_entry.links
        {
            return Err(destination_invalid());
        }
        if path_exists(&self.target)? {
            return Err(destination_exists());
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
enum EntryKind {
    File,
    Directory,
}

struct SourceFile {
    path: PathBuf,
    relative: String,
    entry: StableEntry,
    expected_sha256: Option<String>,
}

struct DirectoryWitness {
    path: PathBuf,
    entry: StableEntry,
}

struct BackupSources {
    files: Vec<SourceFile>,
    directories: Vec<DirectoryWitness>,
    total_bytes: u64,
}

struct DatabaseSnapshot {
    bytes: SqliteOwnedBuf,
    books: HashMap<Uuid, SnapshotBook>,
}

struct SnapshotBook {
    source_path: Option<String>,
    source_sha256: Option<String>,
}

struct SourceCollector<'a> {
    layout: &'a ValidatedAppLayout,
    expected_books: &'a HashMap<Uuid, SnapshotBook>,
    files: Vec<SourceFile>,
    directories: Vec<DirectoryWitness>,
    seen_paths: HashSet<String>,
    seen_files: HashSet<FileIdentity>,
    seen_directories: HashSet<FileIdentity>,
    seen_source_books: HashSet<Uuid>,
    total_bytes: u64,
}

impl<'a> SourceCollector<'a> {
    fn collect(
        layout: &'a ValidatedAppLayout,
        expected_books: &'a HashMap<Uuid, SnapshotBook>,
    ) -> Result<BackupSources, BackupArchiveError> {
        layout.revalidate()?;
        let mut collector = Self {
            layout,
            expected_books,
            files: Vec::new(),
            directories: Vec::new(),
            seen_paths: HashSet::new(),
            seen_files: HashSet::new(),
            seen_directories: HashSet::new(),
            seen_source_books: HashSet::new(),
            total_bytes: 0,
        };
        collector.walk_directory(&layout.books, Path::new(""), 0)?;
        let expected_source_count = expected_books
            .values()
            .filter(|book| book.source_path.is_some())
            .count();
        if collector.seen_source_books.len() != expected_source_count {
            return Err(source_unsafe());
        }
        collector.files.sort_by(|left, right| {
            normalized_token_identity(&left.relative)
                .cmp(&normalized_token_identity(&right.relative))
        });
        Ok(BackupSources {
            files: collector.files,
            directories: collector.directories,
            total_bytes: collector.total_bytes,
        })
    }

    fn walk_directory(
        &mut self,
        directory: &Path,
        relative: &Path,
        depth: usize,
    ) -> Result<(), BackupArchiveError> {
        if depth > MAX_PATH_DEPTH {
            return Err(limit_exceeded());
        }
        validate_source_directory_shape(relative)?;
        if let Some(book_id) = relative_book_id(relative)?
            && self
                .expected_books
                .get(&book_id)
                .is_none_or(|book| book.source_path.is_none())
        {
            return Err(source_unsafe());
        }
        let before = stable_path_entry(directory, EntryKind::Directory)?;
        if self
            .seen_directories
            .len()
            .checked_add(self.files.len())
            .is_none_or(|count| count >= MAX_ENTRY_COUNT)
        {
            return Err(limit_exceeded());
        }
        if before.identity.volume != self.layout.root_entry.identity.volume
            || !self.seen_directories.insert(before.identity)
        {
            return Err(source_unsafe());
        }
        let canonical = fs::canonicalize(directory).map_err(|_| source_unsafe())?;
        if canonical != directory || !path_is_within(&canonical, &self.layout.books) {
            return Err(source_unsafe());
        }

        let mut entries = Vec::new();
        for entry in fs::read_dir(directory).map_err(|_| source_unsafe())? {
            if entries.len() >= MAX_ENTRY_COUNT {
                return Err(limit_exceeded());
            }
            entries.push(entry.map_err(|_| source_unsafe())?);
        }
        entries.sort_by_key(|entry| normalized_os_identity(&entry.file_name()));
        for entry in entries {
            if self.files.len() >= MAX_ENTRY_COUNT.saturating_sub(1)
                || self
                    .seen_directories
                    .len()
                    .checked_add(self.files.len())
                    .is_none_or(|count| count >= MAX_ENTRY_COUNT)
            {
                return Err(limit_exceeded());
            }
            let name = entry
                .file_name()
                .to_str()
                .filter(|name| safe_component(name))
                .ok_or_else(source_unsafe)?
                .to_owned();
            let path = entry.path();
            if path.parent() != Some(directory) || !path_is_within(&path, &self.layout.books) {
                return Err(source_unsafe());
            }
            let child_relative = relative.join(&name);
            let metadata = fs::symlink_metadata(&path).map_err(|_| source_unsafe())?;
            if metadata_is_reparse(&metadata) {
                return Err(source_unsafe());
            }
            if metadata.is_dir() {
                self.walk_directory(&path, &child_relative, depth + 1)?;
            } else if metadata.is_file() {
                let relative_text = manifest_path(&child_relative)?;
                validate_source_file_shape(&relative_text)?;
                let expected_sha256 = if is_source_manifest_path(&relative_text) {
                    let book_id = manifest_book_id(&relative_text)?;
                    let expected = self
                        .expected_books
                        .get(&book_id)
                        .ok_or_else(source_unsafe)?;
                    if expected.source_path.as_deref() != Some(relative_text.as_str())
                        || !self.seen_source_books.insert(book_id)
                    {
                        return Err(source_unsafe());
                    }
                    expected.source_sha256.clone()
                } else {
                    None
                };
                let normalized = normalized_token_identity(&relative_text);
                let stable = stable_path_entry(&path, EntryKind::File)?;
                if stable.identity.volume != self.layout.root_entry.identity.volume
                    || stable.links != 1
                    || !self.seen_paths.insert(normalized)
                    || !self.seen_files.insert(stable.identity)
                {
                    return Err(source_unsafe());
                }
                if stable.len == 0 {
                    return Err(source_unsafe());
                }
                if stable.len > MAX_ENTRY_BYTES {
                    return Err(limit_exceeded());
                }
                self.total_bytes = self
                    .total_bytes
                    .checked_add(stable.len)
                    .filter(|total| *total <= MAX_ARCHIVE_BYTES)
                    .ok_or_else(limit_exceeded)?;
                self.files.push(SourceFile {
                    path,
                    relative: relative_text,
                    entry: stable,
                    expected_sha256,
                });
            } else {
                return Err(source_unsafe());
            }
        }
        require_stable_path(directory, EntryKind::Directory, before)?;
        self.directories.push(DirectoryWitness {
            path: directory.to_path_buf(),
            entry: before,
        });
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct BackupManifest {
    format: String,
    version: u32,
    entry_count: u64,
    total_bytes: u64,
    entries: Vec<BackupManifestEntry>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct BackupManifestEntry {
    pub(crate) path: String,
    pub(crate) offset: u64,
    pub(crate) size: u64,
    pub(crate) sha256: String,
}

pub(crate) struct VerifiedBackup {
    pub(crate) archive_bytes: u64,
    pub(crate) entry_count: u64,
    pub(crate) total_bytes: u64,
    pub(crate) manifest_sha256: String,
    pub(crate) entries: Vec<BackupManifestEntry>,
}

impl fmt::Debug for VerifiedBackup {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VerifiedBackup")
            .field("archive_bytes", &self.archive_bytes)
            .field("entry_count", &self.entry_count)
            .field("total_bytes", &self.total_bytes)
            .field("manifest_sha256", &"<redacted>")
            .field("entries", &self.entries.len())
            .finish()
    }
}

struct TemporaryArchive {
    path: PathBuf,
    file: Option<fs::File>,
    entry: StableEntry,
    published: bool,
}

impl TemporaryArchive {
    fn create(destination: &ValidatedDestination) -> Result<Self, BackupArchiveError> {
        destination.require_missing_and_stable()?;
        for _ in 0..16 {
            let path = destination
                .parent
                .join(format!("{TEMP_FILE_PREFIX}{}.tmp", Uuid::new_v4()));
            let file = match fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(file) => file,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(_) => return Err(write_failed()),
            };
            let entry = match stable_open_file(&path, &file) {
                Ok(entry) => entry,
                Err(_) => {
                    drop(file);
                    let _ = fs::remove_file(&path);
                    return Err(write_failed());
                }
            };
            if entry.identity.volume != destination.parent_entry.identity.volume || entry.links != 1
            {
                drop(file);
                let _ = fs::remove_file(&path);
                return Err(destination_invalid());
            }
            return Ok(Self {
                path,
                file: Some(file),
                entry,
                published: false,
            });
        }
        Err(write_failed())
    }

    fn file_mut(&mut self) -> Result<&mut fs::File, BackupArchiveError> {
        self.file.as_mut().ok_or_else(write_failed)
    }

    fn sync_and_close(&mut self) -> Result<(), BackupArchiveError> {
        let file = self.file.take().ok_or_else(write_failed)?;
        file.sync_all().map_err(|_| write_failed())?;
        drop(file);
        let current = stable_path_entry(&self.path, EntryKind::File).map_err(|_| write_failed())?;
        if current.identity != self.entry.identity || current.links != 1 {
            return Err(write_failed());
        }
        self.entry = current;
        Ok(())
    }

    fn mark_published(&mut self) {
        self.published = true;
    }
}

impl Drop for TemporaryArchive {
    fn drop(&mut self) {
        self.file.take();
        if !self.published {
            let _ = fs::remove_file(&self.path);
        }
    }
}

fn create_archive_file(
    layout: &ValidatedAppLayout,
    destination: &ValidatedDestination,
    database: DatabaseSnapshot,
    fault: &dyn BackupFaultInjector,
) -> Result<BackupSummaryDto, BackupArchiveError> {
    if u64::try_from(database.bytes.len()).map_err(|_| limit_exceeded())? > MAX_DATABASE_BYTES {
        return Err(limit_exceeded());
    }
    let sources = SourceCollector::collect(layout, &database.books)?;
    let database_bytes = u64::try_from(database.bytes.len()).map_err(|_| limit_exceeded())?;
    if database_bytes
        .checked_add(sources.total_bytes)
        .is_none_or(|total| total > MAX_ARCHIVE_BYTES)
    {
        return Err(limit_exceeded());
    }
    checkpoint(fault, BackupBoundary::SourcesValidated)?;
    destination.require_missing_and_stable()?;
    let mut temporary = TemporaryArchive::create(destination)?;

    write_header(temporary.file_mut()?)?;
    checkpoint(fault, BackupBoundary::HeaderWritten)?;
    let mut offset = HEADER_BYTES;
    let mut entries = Vec::with_capacity(sources.files.len().saturating_add(1));
    entries.push(write_bytes_entry(
        temporary.file_mut()?,
        "library.sqlite3",
        database.bytes.as_ref(),
        &mut offset,
    )?);
    for source in &sources.files {
        entries.push(write_source_entry(
            temporary.file_mut()?,
            source,
            &mut offset,
        )?);
    }
    for directory in &sources.directories {
        require_stable_path(&directory.path, EntryKind::Directory, directory.entry)?;
    }
    layout.revalidate()?;
    checkpoint(fault, BackupBoundary::EntriesWritten)?;

    let total_bytes = offset
        .checked_sub(HEADER_BYTES)
        .ok_or_else(limit_exceeded)?;
    if total_bytes > MAX_ARCHIVE_BYTES {
        return Err(limit_exceeded());
    }
    let entry_count = u64::try_from(entries.len()).map_err(|_| limit_exceeded())?;
    let manifest = BackupManifest {
        format: FORMAT_IDENTIFIER.to_owned(),
        version: BACKUP_FORMAT_VERSION,
        entry_count,
        total_bytes,
        entries,
    };
    let manifest_bytes = serde_json::to_vec(&manifest).map_err(|_| write_failed())?;
    let manifest_len = u64::try_from(manifest_bytes.len()).map_err(|_| limit_exceeded())?;
    if manifest_len == 0 || manifest_len > MAX_MANIFEST_BYTES {
        return Err(limit_exceeded());
    }
    temporary
        .file_mut()?
        .write_all(&manifest_bytes)
        .map_err(|_| write_failed())?;
    let manifest_hash = Sha256::digest(&manifest_bytes);
    write_footer(temporary.file_mut()?, manifest_len, &manifest_hash)?;
    temporary.sync_and_close()?;
    sync_directory(&destination.parent)?;
    checkpoint(fault, BackupBoundary::ArchiveSynced)?;

    let verified = verify_archive_file(&temporary.path)?;
    if verified.entry_count != entry_count || verified.total_bytes != total_bytes {
        return Err(verification_failed());
    }
    checkpoint(fault, BackupBoundary::ArchiveVerified)?;
    destination.require_missing_and_stable()?;
    checkpoint(fault, BackupBoundary::BeforePublish)?;
    publish_no_replace(&temporary.path, &destination.target)?;
    temporary.mark_published();
    sync_directory(&destination.parent)?;
    validate_published_file(destination, temporary.entry, verified.archive_bytes)?;

    Ok(BackupSummaryDto {
        format_version: BACKUP_FORMAT_VERSION,
        archive_bytes: verified.archive_bytes,
        entry_count,
    })
}

fn write_header(file: &mut fs::File) -> Result<(), BackupArchiveError> {
    file.write_all(HEADER_MAGIC)
        .and_then(|()| file.write_all(&BACKUP_FORMAT_VERSION.to_le_bytes()))
        .and_then(|()| file.write_all(&0_u32.to_le_bytes()))
        .map_err(|_| write_failed())
}

fn write_footer(
    file: &mut fs::File,
    manifest_len: u64,
    manifest_hash: &[u8],
) -> Result<(), BackupArchiveError> {
    if manifest_hash.len() != 32 {
        return Err(write_failed());
    }
    file.write_all(FOOTER_MAGIC)
        .and_then(|()| file.write_all(&manifest_len.to_le_bytes()))
        .and_then(|()| file.write_all(manifest_hash))
        .map_err(|_| write_failed())
}

fn write_bytes_entry(
    file: &mut fs::File,
    relative: &str,
    bytes: &[u8],
    offset: &mut u64,
) -> Result<BackupManifestEntry, BackupArchiveError> {
    let size = u64::try_from(bytes.len()).map_err(|_| limit_exceeded())?;
    if size > MAX_ENTRY_BYTES {
        return Err(limit_exceeded());
    }
    let entry_offset = *offset;
    file.write_all(bytes).map_err(|_| write_failed())?;
    *offset = offset.checked_add(size).ok_or_else(limit_exceeded)?;
    Ok(BackupManifestEntry {
        path: relative.to_owned(),
        offset: entry_offset,
        size,
        sha256: hex_digest(Sha256::digest(bytes)),
    })
}

fn write_source_entry(
    output: &mut fs::File,
    source: &SourceFile,
    offset: &mut u64,
) -> Result<BackupManifestEntry, BackupArchiveError> {
    require_stable_path(&source.path, EntryKind::File, source.entry)?;
    let mut input = fs::File::open(&source.path).map_err(|_| source_unsafe())?;
    let opened = stable_open_file(&source.path, &input)?;
    if opened != source.entry || opened.links != 1 {
        return Err(source_unsafe());
    }
    let entry_offset = *offset;
    let mut written = 0_u64;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; COPY_BUFFER_BYTES];
    loop {
        let count = input.read(&mut buffer).map_err(|_| source_unsafe())?;
        if count == 0 {
            break;
        }
        written = written
            .checked_add(u64::try_from(count).map_err(|_| limit_exceeded())?)
            .ok_or_else(limit_exceeded)?;
        if written > source.entry.len || written > MAX_ENTRY_BYTES {
            return Err(source_unsafe());
        }
        hasher.update(&buffer[..count]);
        output
            .write_all(&buffer[..count])
            .map_err(|_| write_failed())?;
    }
    if written != source.entry.len {
        return Err(source_unsafe());
    }
    let after = stable_open_file(&source.path, &input)?;
    if after != source.entry {
        return Err(source_unsafe());
    }
    drop(input);
    require_stable_path(&source.path, EntryKind::File, source.entry)?;
    *offset = offset.checked_add(written).ok_or_else(limit_exceeded)?;
    let sha256 = hex_digest(hasher.finalize());
    if source
        .expected_sha256
        .as_ref()
        .is_some_and(|expected| expected != &sha256)
    {
        return Err(source_unsafe());
    }
    Ok(BackupManifestEntry {
        path: source.relative.clone(),
        offset: entry_offset,
        size: written,
        sha256,
    })
}

pub(crate) fn verify_archive_file(path: &Path) -> Result<VerifiedBackup, BackupArchiveError> {
    let canonical_path = canonical_existing_file(path).map_err(|_| verification_failed())?;
    let path = canonical_path.as_path();
    let path_entry = stable_path_entry(path, EntryKind::File).map_err(|_| verification_failed())?;
    if path_entry.links != 1 {
        return Err(verification_failed());
    }
    let minimum = HEADER_BYTES
        .checked_add(FOOTER_BYTES)
        .and_then(|value| value.checked_add(1))
        .ok_or_else(verification_failed)?;
    let maximum = HEADER_BYTES
        .checked_add(MAX_ARCHIVE_BYTES)
        .and_then(|value| value.checked_add(MAX_MANIFEST_BYTES))
        .and_then(|value| value.checked_add(FOOTER_BYTES))
        .ok_or_else(verification_failed)?;
    if path_entry.len < minimum || path_entry.len > maximum {
        return Err(verification_failed());
    }
    let mut file = fs::File::open(path).map_err(|_| verification_failed())?;
    if stable_open_file(path, &file).map_err(|_| verification_failed())? != path_entry {
        return Err(verification_failed());
    }
    let mut magic = [0_u8; 8];
    file.read_exact(&mut magic)
        .map_err(|_| verification_failed())?;
    let version = read_u32(&mut file)?;
    let flags = read_u32(&mut file)?;
    if &magic != HEADER_MAGIC || version != BACKUP_FORMAT_VERSION || flags != 0 {
        return Err(verification_failed());
    }

    let footer_offset = path_entry
        .len
        .checked_sub(FOOTER_BYTES)
        .ok_or_else(verification_failed)?;
    file.seek(SeekFrom::Start(footer_offset))
        .map_err(|_| verification_failed())?;
    let mut footer_magic = [0_u8; 8];
    file.read_exact(&mut footer_magic)
        .map_err(|_| verification_failed())?;
    let manifest_len = read_u64(&mut file)?;
    let mut expected_manifest_hash = [0_u8; 32];
    file.read_exact(&mut expected_manifest_hash)
        .map_err(|_| verification_failed())?;
    if &footer_magic != FOOTER_MAGIC || manifest_len == 0 || manifest_len > MAX_MANIFEST_BYTES {
        return Err(verification_failed());
    }
    let manifest_offset = footer_offset
        .checked_sub(manifest_len)
        .filter(|offset| *offset >= HEADER_BYTES)
        .ok_or_else(verification_failed)?;
    file.seek(SeekFrom::Start(manifest_offset))
        .map_err(|_| verification_failed())?;
    let manifest_size = usize::try_from(manifest_len).map_err(|_| verification_failed())?;
    let mut manifest_bytes = vec![0_u8; manifest_size];
    file.read_exact(&mut manifest_bytes)
        .map_err(|_| verification_failed())?;
    if Sha256::digest(&manifest_bytes).as_slice() != expected_manifest_hash {
        return Err(verification_failed());
    }
    let manifest: BackupManifest =
        serde_json::from_slice(&manifest_bytes).map_err(|_| verification_failed())?;
    validate_manifest(&manifest, manifest_offset)?;

    let mut buffer = vec![0_u8; COPY_BUFFER_BYTES];
    for entry in &manifest.entries {
        file.seek(SeekFrom::Start(entry.offset))
            .map_err(|_| verification_failed())?;
        let mut remaining = entry.size;
        let mut hasher = Sha256::new();
        while remaining > 0 {
            let count = usize::try_from(remaining.min(COPY_BUFFER_BYTES as u64))
                .map_err(|_| verification_failed())?;
            file.read_exact(&mut buffer[..count])
                .map_err(|_| verification_failed())?;
            hasher.update(&buffer[..count]);
            remaining = remaining
                .checked_sub(u64::try_from(count).map_err(|_| verification_failed())?)
                .ok_or_else(verification_failed)?;
        }
        if hex_digest(hasher.finalize()) != entry.sha256 {
            return Err(verification_failed());
        }
    }
    if stable_open_file(path, &file).map_err(|_| verification_failed())? != path_entry {
        return Err(verification_failed());
    }
    drop(file);
    require_stable_path(path, EntryKind::File, path_entry).map_err(|_| verification_failed())?;
    Ok(VerifiedBackup {
        archive_bytes: path_entry.len,
        entry_count: manifest.entry_count,
        total_bytes: manifest.total_bytes,
        manifest_sha256: hex_digest(expected_manifest_hash),
        entries: manifest.entries,
    })
}

/// Copies one user-selected archive into an already-created app-owned staging
/// directory before parsing it. Both the source path and open handle are
/// witnessed so a reparse/hard-link/race cannot switch the bytes under us.
pub(crate) fn copy_archive_into_staging(
    source: &Path,
    staged_archive: &Path,
    app_root: &Path,
) -> Result<VerifiedBackup, BackupArchiveError> {
    validate_local_absolute_path(source).map_err(|_| verification_failed())?;
    if source
        .extension()
        .and_then(OsStr::to_str)
        .is_none_or(|extension| !extension.eq_ignore_ascii_case("tlbackup"))
    {
        return Err(verification_failed());
    }
    let canonical_source = canonical_existing_file(source)?;
    if path_is_within(&canonical_source, app_root) {
        return Err(verification_failed());
    }
    let source_entry =
        stable_path_entry(&canonical_source, EntryKind::File).map_err(|_| verification_failed())?;
    if source_entry.links != 1 {
        return Err(verification_failed());
    }
    let maximum = HEADER_BYTES
        .checked_add(MAX_ARCHIVE_BYTES)
        .and_then(|value| value.checked_add(MAX_MANIFEST_BYTES))
        .and_then(|value| value.checked_add(FOOTER_BYTES))
        .ok_or_else(verification_failed)?;
    if source_entry.len > maximum {
        return Err(limit_exceeded());
    }

    let staged_parent = staged_archive.parent().ok_or_else(write_failed)?;
    let parent_entry =
        stable_path_entry(staged_parent, EntryKind::Directory).map_err(|_| write_failed())?;
    if !path_is_within(staged_parent, app_root) || path_exists(staged_archive)? {
        return Err(write_failed());
    }
    let mut input = fs::File::open(&canonical_source).map_err(|_| verification_failed())?;
    if stable_open_file(&canonical_source, &input).map_err(|_| verification_failed())?
        != source_entry
    {
        return Err(verification_failed());
    }
    let mut output = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(staged_archive)
        .map_err(|_| write_failed())?;
    let created = stable_open_file(staged_archive, &output).map_err(|_| write_failed())?;
    if created.links != 1 || created.identity.volume != parent_entry.identity.volume {
        drop(output);
        let _ = fs::remove_file(staged_archive);
        return Err(write_failed());
    }

    let copied = (|| {
        let mut total = 0_u64;
        let mut buffer = vec![0_u8; COPY_BUFFER_BYTES];
        loop {
            let count = input.read(&mut buffer).map_err(|_| verification_failed())?;
            if count == 0 {
                break;
            }
            total = total
                .checked_add(u64::try_from(count).map_err(|_| limit_exceeded())?)
                .filter(|value| *value <= maximum)
                .ok_or_else(limit_exceeded)?;
            if total > source_entry.len {
                return Err(verification_failed());
            }
            output
                .write_all(&buffer[..count])
                .map_err(|_| write_failed())?;
        }
        if total != source_entry.len
            || stable_open_file(&canonical_source, &input).map_err(|_| verification_failed())?
                != source_entry
        {
            return Err(verification_failed());
        }
        output.sync_all().map_err(|_| write_failed())?;
        let staged_entry = stable_open_file(staged_archive, &output).map_err(|_| write_failed())?;
        if staged_entry.identity != created.identity
            || staged_entry.len != source_entry.len
            || staged_entry.links != 1
        {
            return Err(write_failed());
        }
        drop(output);
        require_stable_path(&canonical_source, EntryKind::File, source_entry)
            .map_err(|_| verification_failed())?;
        require_stable_path(staged_archive, EntryKind::File, staged_entry)
            .map_err(|_| write_failed())?;
        sync_directory(staged_parent)?;
        verify_archive_file(staged_archive)
    })();
    if copied.is_err() {
        drop(input);
        let _ = fs::remove_file(staged_archive);
        let _ = sync_directory(staged_parent);
    }
    copied
}

/// Expands a previously verified TLBACKUP into a fresh app-owned directory.
/// Paths are reconstructed only from the canonical manifest grammar; no
/// archive-provided absolute path is ever accepted.
pub(crate) fn extract_verified_archive(
    archive: &Path,
    verified: &VerifiedBackup,
    dataset_root: &Path,
) -> Result<(), BackupArchiveError> {
    let archive_entry =
        stable_path_entry(archive, EntryKind::File).map_err(|_| verification_failed())?;
    if archive_entry.links != 1 || archive_entry.len != verified.archive_bytes {
        return Err(verification_failed());
    }
    let dataset_entry =
        stable_path_entry(dataset_root, EntryKind::Directory).map_err(|_| write_failed())?;
    if fs::read_dir(dataset_root)
        .map_err(|_| write_failed())?
        .next()
        .is_some()
    {
        return Err(write_failed());
    }
    let mut input = fs::File::open(archive).map_err(|_| verification_failed())?;
    if stable_open_file(archive, &input).map_err(|_| verification_failed())? != archive_entry {
        return Err(verification_failed());
    }
    let mut created_directories = HashSet::new();
    created_directories.insert(normalized_path_identity(dataset_root));
    let mut buffer = vec![0_u8; COPY_BUFFER_BYTES];
    for entry in &verified.entries {
        validate_manifest_path(&entry.path)?;
        let components = entry.path.split('/').collect::<Vec<_>>();
        let mut parent = dataset_root.to_path_buf();
        for component in &components[..components.len().saturating_sub(1)] {
            parent.push(component);
            let identity = normalized_path_identity(&parent);
            if created_directories.insert(identity) {
                fs::create_dir(&parent).map_err(|_| write_failed())?;
                let current =
                    stable_path_entry(&parent, EntryKind::Directory).map_err(|_| write_failed())?;
                if current.links == 0 || current.identity.volume != dataset_entry.identity.volume {
                    return Err(write_failed());
                }
                sync_directory(parent.parent().ok_or_else(write_failed)?)?;
            } else {
                stable_path_entry(&parent, EntryKind::Directory).map_err(|_| write_failed())?;
            }
        }
        let file_name = components.last().ok_or_else(verification_failed)?;
        let target = parent.join(file_name);
        if target.parent() != Some(parent.as_path()) || !path_is_within(&target, dataset_root) {
            return Err(verification_failed());
        }
        let mut output = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&target)
            .map_err(|_| write_failed())?;
        let created = stable_open_file(&target, &output).map_err(|_| write_failed())?;
        if created.links != 1 || created.identity.volume != dataset_entry.identity.volume {
            return Err(write_failed());
        }
        input
            .seek(SeekFrom::Start(entry.offset))
            .map_err(|_| verification_failed())?;
        let mut remaining = entry.size;
        let mut hasher = Sha256::new();
        while remaining > 0 {
            let count = usize::try_from(remaining.min(COPY_BUFFER_BYTES as u64))
                .map_err(|_| verification_failed())?;
            input
                .read_exact(&mut buffer[..count])
                .map_err(|_| verification_failed())?;
            output
                .write_all(&buffer[..count])
                .map_err(|_| write_failed())?;
            hasher.update(&buffer[..count]);
            remaining = remaining
                .checked_sub(u64::try_from(count).map_err(|_| verification_failed())?)
                .ok_or_else(verification_failed)?;
        }
        if hex_digest(hasher.finalize()) != entry.sha256 {
            return Err(verification_failed());
        }
        output.sync_all().map_err(|_| write_failed())?;
        let after = stable_open_file(&target, &output).map_err(|_| write_failed())?;
        if after.identity != created.identity || after.len != entry.size || after.links != 1 {
            return Err(write_failed());
        }
        drop(output);
        require_stable_path(&target, EntryKind::File, after).map_err(|_| write_failed())?;
        sync_directory(&parent)?;
    }
    if stable_open_file(archive, &input).map_err(|_| verification_failed())? != archive_entry {
        return Err(verification_failed());
    }
    require_stable_path(archive, EntryKind::File, archive_entry)
        .map_err(|_| verification_failed())?;
    let dataset_after =
        stable_path_entry(dataset_root, EntryKind::Directory).map_err(|_| write_failed())?;
    if dataset_after.identity != dataset_entry.identity || dataset_after.links == 0 {
        return Err(write_failed());
    }
    sync_directory(dataset_root)
}

pub(crate) fn verify_materialized_file(
    path: &Path,
    expected_size: u64,
    expected_sha256: &str,
) -> Result<(), BackupArchiveError> {
    let expected = stable_path_entry(path, EntryKind::File).map_err(|_| verification_failed())?;
    if expected.links != 1 || expected.len != expected_size {
        return Err(verification_failed());
    }
    let mut file = fs::File::open(path).map_err(|_| verification_failed())?;
    if stable_open_file(path, &file).map_err(|_| verification_failed())? != expected {
        return Err(verification_failed());
    }
    let mut remaining = expected_size;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; COPY_BUFFER_BYTES];
    while remaining > 0 {
        let count = usize::try_from(remaining.min(COPY_BUFFER_BYTES as u64))
            .map_err(|_| verification_failed())?;
        file.read_exact(&mut buffer[..count])
            .map_err(|_| verification_failed())?;
        hasher.update(&buffer[..count]);
        remaining = remaining
            .checked_sub(u64::try_from(count).map_err(|_| verification_failed())?)
            .ok_or_else(verification_failed)?;
    }
    let mut trailing = [0_u8; 1];
    if file
        .read(&mut trailing)
        .map_err(|_| verification_failed())?
        != 0
        || hex_digest(hasher.finalize()) != expected_sha256
        || stable_open_file(path, &file).map_err(|_| verification_failed())? != expected
    {
        return Err(verification_failed());
    }
    drop(file);
    require_stable_path(path, EntryKind::File, expected).map_err(|_| verification_failed())
}

pub(crate) fn require_safe_directory(path: &Path) -> Result<(), BackupArchiveError> {
    stable_path_entry(path, EntryKind::Directory)
        .map(|_| ())
        .map_err(|_| verification_failed())
}

pub(crate) fn safe_path_exists(path: &Path) -> Result<bool, BackupArchiveError> {
    path_exists(path)
}

pub(crate) fn sync_app_directory(path: &Path) -> Result<(), BackupArchiveError> {
    sync_directory(path)
}

fn canonical_existing_file(path: &Path) -> Result<PathBuf, BackupArchiveError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| verification_failed())?;
    if metadata_is_reparse(&metadata) || !metadata.is_file() {
        return Err(verification_failed());
    }
    let requested_parent = path.parent().ok_or_else(verification_failed)?;
    ensure_no_reparse_ancestors(requested_parent).map_err(|_| verification_failed())?;
    let canonical_parent = fs::canonicalize(requested_parent).map_err(|_| verification_failed())?;
    let canonical = fs::canonicalize(path).map_err(|_| verification_failed())?;
    if canonical.parent() != Some(canonical_parent.as_path())
        || canonical.file_name() != path.file_name()
    {
        return Err(verification_failed());
    }
    Ok(canonical)
}

fn validate_manifest(
    manifest: &BackupManifest,
    manifest_offset: u64,
) -> Result<(), BackupArchiveError> {
    if manifest.format != FORMAT_IDENTIFIER
        || manifest.version != BACKUP_FORMAT_VERSION
        || manifest.entries.is_empty()
        || manifest.entries.len() > MAX_ENTRY_COUNT
        || manifest.entry_count
            != u64::try_from(manifest.entries.len()).map_err(|_| verification_failed())?
        || manifest.entries[0].path != "library.sqlite3"
    {
        return Err(verification_failed());
    }
    let mut expected_offset = HEADER_BYTES;
    let mut total_bytes = 0_u64;
    let mut seen = HashSet::new();
    let mut database_count = 0_u8;
    for entry in &manifest.entries {
        validate_manifest_path(&entry.path)?;
        if entry.path == "library.sqlite3" {
            database_count = database_count
                .checked_add(1)
                .ok_or_else(verification_failed)?;
            if entry.size == 0 || entry.size > MAX_DATABASE_BYTES {
                return Err(verification_failed());
            }
        } else if entry.size == 0 || entry.size > MAX_ENTRY_BYTES {
            return Err(verification_failed());
        }
        if entry.offset != expected_offset
            || entry.sha256.len() != 64
            || !entry
                .sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
            || !seen.insert(normalized_token_identity(&entry.path))
        {
            return Err(verification_failed());
        }
        expected_offset = expected_offset
            .checked_add(entry.size)
            .ok_or_else(verification_failed)?;
        total_bytes = total_bytes
            .checked_add(entry.size)
            .ok_or_else(verification_failed)?;
    }
    if database_count != 1
        || expected_offset != manifest_offset
        || total_bytes != manifest.total_bytes
        || total_bytes > MAX_ARCHIVE_BYTES
    {
        return Err(verification_failed());
    }
    Ok(())
}

async fn sanitized_database_snapshot(
    pool: &SqlitePool,
) -> Result<DatabaseSnapshot, BackupArchiveError> {
    let mut source = pool.acquire().await.map_err(|_| snapshot_failed())?;
    sqlx::query("BEGIN")
        .execute(&mut *source)
        .await
        .map_err(|_| snapshot_failed())?;
    let serialized = async {
        require_database_integrity(&mut source).await?;
        source.serialize(None).await.map_err(|_| snapshot_failed())
    }
    .await;
    let rollback = sqlx::query("ROLLBACK").execute(&mut *source).await;
    let mut bytes = serialized?;
    rollback.map_err(|_| snapshot_failed())?;
    normalize_sqlite_image(&mut bytes)?;

    let mut sanitized = SqliteConnection::connect("sqlite::memory:")
        .await
        .map_err(|_| snapshot_failed())?;
    sanitized
        .deserialize(None, bytes, false)
        .await
        .map_err(|_| snapshot_failed())?;
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&mut sanitized)
        .await
        .map_err(|_| snapshot_failed())?;
    sqlx::query("PRAGMA secure_delete = ON")
        .execute(&mut sanitized)
        .await
        .map_err(|_| snapshot_failed())?;

    let trigger_sql = sqlx::query_scalar::<_, String>(
        "SELECT sql FROM sqlite_schema WHERE type = 'trigger' AND name = 'provider_remote_resources_delete_only_resolved'",
    )
    .fetch_optional(&mut sanitized)
    .await
    .map_err(|_| snapshot_failed())?
    .ok_or_else(snapshot_failed)?;
    if normalized_sql(&trigger_sql) != normalized_sql(REMOTE_RESOURCE_DELETE_TRIGGER) {
        return Err(snapshot_failed());
    }
    let failed_migrations: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM _sqlx_migrations WHERE success = 0")
            .fetch_one(&mut sanitized)
            .await
            .map_err(|_| snapshot_failed())?;
    if failed_migrations != 0 {
        return Err(snapshot_failed());
    }
    sqlx::query("DROP TRIGGER provider_remote_resources_delete_only_resolved")
        .execute(&mut sanitized)
        .await
        .map_err(|_| snapshot_failed())?;
    sqlx::query("DELETE FROM provider_remote_resources")
        .execute(&mut sanitized)
        .await
        .map_err(|_| snapshot_failed())?;
    sqlx::query(REMOTE_RESOURCE_DELETE_TRIGGER)
        .execute(&mut sanitized)
        .await
        .map_err(|_| snapshot_failed())?;
    sqlx::query("VACUUM")
        .execute(&mut sanitized)
        .await
        .map_err(|_| snapshot_failed())?;
    require_database_integrity(&mut sanitized).await?;
    let remote_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM provider_remote_resources")
        .fetch_one(&mut sanitized)
        .await
        .map_err(|_| snapshot_failed())?;
    let forbidden_columns: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pragma_table_info('provider_profiles') WHERE lower(name) IN ('credential_key', 'api_key', 'secret', 'token')",
    )
    .fetch_one(&mut sanitized)
    .await
    .map_err(|_| snapshot_failed())?;
    if remote_count != 0 || forbidden_columns != 0 {
        return Err(snapshot_failed());
    }
    let books = load_snapshot_books(&mut sanitized).await?;
    let mut result = sanitized
        .serialize(None)
        .await
        .map_err(|_| snapshot_failed())?;
    normalize_sqlite_image(&mut result)?;
    Ok(DatabaseSnapshot {
        bytes: result,
        books,
    })
}

async fn load_snapshot_books(
    connection: &mut SqliteConnection,
) -> Result<HashMap<Uuid, SnapshotBook>, BackupArchiveError> {
    let limit = i64::try_from(MAX_ENTRY_COUNT)
        .ok()
        .and_then(|value| value.checked_add(1))
        .ok_or_else(limit_exceeded)?;
    let rows = sqlx::query("SELECT id, sha256, format, stored_path FROM books ORDER BY id LIMIT ?")
        .bind(limit)
        .fetch_all(&mut *connection)
        .await
        .map_err(|_| snapshot_failed())?;
    if rows.len() > MAX_ENTRY_COUNT {
        return Err(limit_exceeded());
    }
    let mut books = HashMap::with_capacity(rows.len());
    for row in rows {
        let id_text: String = row.try_get("id").map_err(|_| snapshot_failed())?;
        let id = parse_canonical_uuid(&id_text).ok_or_else(snapshot_failed)?;
        let format: String = row.try_get("format").map_err(|_| snapshot_failed())?;
        let sha256: Option<String> = row.try_get("sha256").map_err(|_| snapshot_failed())?;
        let stored_path: Option<String> =
            row.try_get("stored_path").map_err(|_| snapshot_failed())?;
        let expected_path = match format.as_str() {
            "pdf" | "epub" | "docx" => format!("books/{id}/original.{format}"),
            _ => return Err(snapshot_failed()),
        };
        match (&sha256, &stored_path) {
            (Some(hash), Some(path))
                if path == &expected_path
                    && hash.len() == 64
                    && hash
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f')) => {}
            (None, None) => {}
            _ => return Err(snapshot_failed()),
        }
        if books
            .insert(
                id,
                SnapshotBook {
                    source_path: stored_path,
                    source_sha256: sha256,
                },
            )
            .is_some()
        {
            return Err(snapshot_failed());
        }
    }
    Ok(books)
}

async fn require_database_integrity(
    connection: &mut SqliteConnection,
) -> Result<(), BackupArchiveError> {
    let results = sqlx::query_scalar::<_, String>("PRAGMA integrity_check")
        .fetch_all(&mut *connection)
        .await
        .map_err(|_| snapshot_failed())?;
    if results.as_slice() != ["ok"] {
        return Err(snapshot_failed());
    }
    if sqlx::query("PRAGMA foreign_key_check")
        .fetch_optional(&mut *connection)
        .await
        .map_err(|_| snapshot_failed())?
        .is_some()
    {
        return Err(snapshot_failed());
    }
    Ok(())
}

fn normalize_sqlite_image(bytes: &mut SqliteOwnedBuf) -> Result<(), BackupArchiveError> {
    let length = u64::try_from(bytes.len()).map_err(|_| limit_exceeded())?;
    if !(100..=MAX_DATABASE_BYTES).contains(&length) || &bytes[..16] != b"SQLite format 3\0" {
        return Err(snapshot_failed());
    }
    let raw_page_size = u16::from_be_bytes([bytes[16], bytes[17]]);
    let page_size = if raw_page_size == 1 {
        65_536_u64
    } else {
        u64::from(raw_page_size)
    };
    if !(512..=65_536).contains(&page_size)
        || !page_size.is_power_of_two()
        || !length.is_multiple_of(page_size)
    {
        return Err(snapshot_failed());
    }
    bytes[18] = 1;
    bytes[19] = 1;
    Ok(())
}

fn validate_source_directory_shape(relative: &Path) -> Result<(), BackupArchiveError> {
    if relative.as_os_str().is_empty() {
        return Ok(());
    }
    let components = relative
        .components()
        .map(normal_component_text)
        .collect::<Result<Vec<_>, _>>()?;
    if components.is_empty()
        || components.len() > MAX_PATH_DEPTH
        || parse_canonical_uuid(components[0]).is_none()
        || components.len() > 2
        || (components.len() == 2 && components[1] != "derived")
    {
        return Err(source_unsafe());
    }
    Ok(())
}

fn relative_book_id(relative: &Path) -> Result<Option<Uuid>, BackupArchiveError> {
    let Some(component) = relative.components().next() else {
        return Ok(None);
    };
    let value = normal_component_text(component)?;
    parse_canonical_uuid(value)
        .map(Some)
        .ok_or_else(source_unsafe)
}

fn validate_source_file_shape(relative: &str) -> Result<(), BackupArchiveError> {
    validate_backup_entry_path(relative, false).map_err(|_| source_unsafe())
}

fn validate_manifest_path(relative: &str) -> Result<(), BackupArchiveError> {
    validate_backup_entry_path(relative, true).map_err(|_| verification_failed())
}

fn validate_backup_entry_path(
    relative: &str,
    allow_database: bool,
) -> Result<(), BackupArchiveError> {
    if relative == "library.sqlite3" {
        return if allow_database {
            Ok(())
        } else {
            Err(source_unsafe())
        };
    }
    if relative.is_empty()
        || relative.len() > MAX_RELATIVE_PATH_BYTES
        || relative.starts_with('/')
        || relative.ends_with('/')
        || relative.contains(['\\', '\0'])
        || relative.contains("//")
    {
        return Err(source_unsafe());
    }
    let components = relative.split('/').collect::<Vec<_>>();
    if components.len() < 3
        || components.len() > MAX_PATH_DEPTH
        || components[0] != "books"
        || parse_canonical_uuid(components[1]).is_none()
        || components
            .iter()
            .any(|component| !safe_component(component))
    {
        return Err(source_unsafe());
    }
    if components.len() == 3
        && matches!(
            components[2],
            "original.pdf" | "original.epub" | "original.docx"
        )
    {
        return Ok(());
    }
    if components.len() == 4 && components[2] == "derived" && components[3] == "document.html" {
        return Ok(());
    }
    Err(source_unsafe())
}

fn is_source_manifest_path(relative: &str) -> bool {
    relative
        .rsplit_once('/')
        .is_some_and(|(_, name)| matches!(name, "original.pdf" | "original.epub" | "original.docx"))
}

fn manifest_book_id(relative: &str) -> Result<Uuid, BackupArchiveError> {
    relative
        .split('/')
        .nth(1)
        .and_then(parse_canonical_uuid)
        .ok_or_else(source_unsafe)
}

fn manifest_path(relative_to_books: &Path) -> Result<String, BackupArchiveError> {
    let mut text = String::from("books");
    for component in relative_to_books.components() {
        let value = normal_component_text(component)?;
        if !safe_component(value) {
            return Err(source_unsafe());
        }
        text.push('/');
        text.push_str(value);
    }
    if text.len() > MAX_RELATIVE_PATH_BYTES {
        return Err(limit_exceeded());
    }
    Ok(text)
}

fn normal_component_text(component: Component<'_>) -> Result<&str, BackupArchiveError> {
    match component {
        Component::Normal(value) => value.to_str().ok_or_else(source_unsafe),
        _ => Err(source_unsafe()),
    }
}

fn safe_component(component: &str) -> bool {
    !component.is_empty()
        && component != "."
        && component != ".."
        && !component.ends_with(['.', ' '])
        && !component.contains(['\\', '/', ':', '\0'])
        && !component
            .chars()
            .any(|character| character.is_ascii_control() || "<>\"|?*[]".contains(character))
        && !component.contains("${")
        && !component.to_ascii_lowercase().contains("$env:")
        && !contains_percent_environment_reference(component)
        && !is_windows_device_name(component)
}

fn contains_percent_environment_reference(value: &str) -> bool {
    let mut indexes = value.match_indices('%');
    indexes.next().is_some() && indexes.next().is_some()
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

fn validate_local_absolute_path(path: &Path) -> Result<(), BackupArchiveError> {
    if path.as_os_str().is_empty()
        || !path.is_absolute()
        || path.as_os_str().to_string_lossy().len() > MAX_DESTINATION_PATH_BYTES
        || path
            .components()
            .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
    {
        return Err(destination_invalid());
    }
    let text = path.as_os_str().to_string_lossy();
    if text.contains('*')
        || text.contains('?')
        || text.contains('[')
        || text.contains(']')
        || text.contains("${")
        || text.to_ascii_lowercase().contains("$env:")
        || contains_percent_environment_reference(&text)
    {
        return Err(destination_invalid());
    }
    #[cfg(windows)]
    {
        use std::path::Prefix;
        if !matches!(
            path.components().next(),
            Some(Component::Prefix(prefix)) if matches!(prefix.kind(), Prefix::Disk(_))
        ) {
            return Err(destination_invalid());
        }
    }
    Ok(())
}

fn validate_canonical_local_path(path: &Path) -> Result<(), BackupArchiveError> {
    if path.as_os_str().is_empty()
        || !path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
    {
        return Err(source_unsafe());
    }
    #[cfg(windows)]
    {
        use std::path::Prefix;
        if !matches!(
            path.components().next(),
            Some(Component::Prefix(prefix))
                if matches!(prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_))
        ) {
            return Err(source_unsafe());
        }
    }
    Ok(())
}

fn ensure_no_reparse_ancestors(path: &Path) -> Result<(), BackupArchiveError> {
    for ancestor in path.ancestors() {
        if ancestor.as_os_str().is_empty() {
            continue;
        }
        let metadata = fs::symlink_metadata(ancestor).map_err(|_| destination_invalid())?;
        if metadata_is_reparse(&metadata) || !metadata.is_dir() {
            return Err(destination_invalid());
        }
    }
    Ok(())
}

fn stable_path_entry(path: &Path, kind: EntryKind) -> Result<StableEntry, BackupArchiveError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| source_unsafe())?;
    if metadata_is_reparse(&metadata) || !metadata_matches_kind(&metadata, kind) {
        return Err(source_unsafe());
    }
    let canonical = fs::canonicalize(path).map_err(|_| source_unsafe())?;
    if canonical != path {
        return Err(source_unsafe());
    }
    let info = path_file_information(path)?;
    Ok(StableEntry {
        identity: info.identity,
        len: metadata.len(),
        modified: metadata.modified().map_err(|_| source_unsafe())?,
        links: info.links,
    })
}

fn stable_open_file(path: &Path, file: &fs::File) -> Result<StableEntry, BackupArchiveError> {
    let metadata = file.metadata().map_err(|_| source_unsafe())?;
    if !metadata.is_file() {
        return Err(source_unsafe());
    }
    let info = open_file_information(file)?;
    let path_metadata = fs::symlink_metadata(path).map_err(|_| source_unsafe())?;
    if metadata_is_reparse(&path_metadata) || !path_metadata.is_file() {
        return Err(source_unsafe());
    }
    let path_info = path_file_information(path)?;
    if info.identity != path_info.identity || info.links != path_info.links {
        return Err(source_unsafe());
    }
    Ok(StableEntry {
        identity: info.identity,
        len: metadata.len(),
        modified: metadata.modified().map_err(|_| source_unsafe())?,
        links: info.links,
    })
}

fn require_stable_path(
    path: &Path,
    kind: EntryKind,
    expected: StableEntry,
) -> Result<(), BackupArchiveError> {
    if stable_path_entry(path, kind)? != expected {
        return Err(source_unsafe());
    }
    Ok(())
}

fn metadata_matches_kind(metadata: &fs::Metadata, kind: EntryKind) -> bool {
    match kind {
        EntryKind::File => metadata.is_file(),
        EntryKind::Directory => metadata.is_dir(),
    }
}

#[derive(Clone, Copy)]
struct FileInformation {
    identity: FileIdentity,
    links: u64,
}

#[cfg(unix)]
fn path_file_information(path: &Path) -> Result<FileInformation, BackupArchiveError> {
    use std::os::unix::fs::MetadataExt;
    let metadata = fs::symlink_metadata(path).map_err(|_| source_unsafe())?;
    Ok(FileInformation {
        identity: FileIdentity {
            volume: metadata.dev(),
            index: metadata.ino(),
        },
        links: metadata.nlink(),
    })
}

#[cfg(unix)]
fn open_file_information(file: &fs::File) -> Result<FileInformation, BackupArchiveError> {
    use std::os::unix::fs::MetadataExt;
    let metadata = file.metadata().map_err(|_| source_unsafe())?;
    Ok(FileInformation {
        identity: FileIdentity {
            volume: metadata.dev(),
            index: metadata.ino(),
        },
        links: metadata.nlink(),
    })
}

#[cfg(windows)]
fn path_file_information(path: &Path) -> Result<FileInformation, BackupArchiveError> {
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
    // SAFETY: `wide` is live and NUL-terminated; optional pointers are null.
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
        return Err(source_unsafe());
    }
    let information = windows_handle_information(handle);
    // SAFETY: `handle` was returned by CreateFileW and is closed exactly once.
    let _ = unsafe { close_handle(handle) };
    information
}

#[cfg(windows)]
fn open_file_information(file: &fs::File) -> Result<FileInformation, BackupArchiveError> {
    use std::os::windows::io::AsRawHandle;
    windows_handle_information(file.as_raw_handle().cast())
}

#[cfg(windows)]
fn windows_handle_information(
    handle: *mut std::ffi::c_void,
) -> Result<FileInformation, BackupArchiveError> {
    // SAFETY: the all-integer C representation admits an all-zero value.
    let mut information: WindowsByHandleFileInformation = unsafe { std::mem::zeroed() };
    // SAFETY: `handle` is valid for the duration of the call and the output is writable.
    if unsafe { get_file_information_by_handle(handle, &mut information) } == 0 {
        return Err(source_unsafe());
    }
    Ok(FileInformation {
        identity: FileIdentity {
            volume: u64::from(information.volume_serial_number),
            index: (u64::from(information.file_index_high) << 32)
                | u64::from(information.file_index_low),
        },
        links: u64::from(information.number_of_links),
    })
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

#[cfg(not(any(unix, windows)))]
fn path_file_information(_path: &Path) -> Result<FileInformation, BackupArchiveError> {
    Err(source_unsafe())
}

#[cfg(not(any(unix, windows)))]
fn open_file_information(_file: &fs::File) -> Result<FileInformation, BackupArchiveError> {
    Err(source_unsafe())
}

fn validate_published_file(
    destination: &ValidatedDestination,
    expected: StableEntry,
    archive_bytes: u64,
) -> Result<(), BackupArchiveError> {
    let actual =
        stable_path_entry(&destination.target, EntryKind::File).map_err(|_| write_failed())?;
    if actual.identity != expected.identity
        || actual.len != archive_bytes
        || actual.links != 1
        || actual.identity.volume != destination.parent_entry.identity.volume
    {
        return Err(write_failed());
    }
    Ok(())
}

#[cfg(windows)]
fn publish_no_replace(source: &Path, destination: &Path) -> Result<(), BackupArchiveError> {
    use std::os::windows::ffi::OsStrExt;
    const MOVEFILE_WRITE_THROUGH: u32 = 0x0000_0008;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        #[link_name = "MoveFileExW"]
        fn move_file_ex_w(existing: *const u16, target: *const u16, flags: u32) -> i32;
    }
    if path_exists(destination)? {
        return Err(destination_exists());
    }
    let mut source_wide = source.as_os_str().encode_wide().collect::<Vec<_>>();
    source_wide.push(0);
    let mut destination_wide = destination.as_os_str().encode_wide().collect::<Vec<_>>();
    destination_wide.push(0);
    // SAFETY: both buffers are live NUL-terminated UTF-16 strings. The replace flag is absent.
    if unsafe {
        move_file_ex_w(
            source_wide.as_ptr(),
            destination_wide.as_ptr(),
            MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        if path_exists(destination)? {
            return Err(destination_exists());
        }
        return Err(write_failed());
    }
    Ok(())
}

#[cfg(not(windows))]
fn publish_no_replace(source: &Path, destination: &Path) -> Result<(), BackupArchiveError> {
    fs::hard_link(source, destination).map_err(|error| {
        if error.kind() == std::io::ErrorKind::AlreadyExists {
            destination_exists()
        } else {
            write_failed()
        }
    })?;
    fs::remove_file(source).map_err(|_| write_failed())
}

#[cfg(windows)]
fn sync_directory(path: &Path) -> Result<(), BackupArchiveError> {
    use std::os::windows::fs::OpenOptionsExt;

    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    let directory = fs::OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)
        .map_err(|_| write_failed())?;
    // FlushFileBuffers does not support Windows directory handles. Opening the
    // exact directory validates it; payload durability comes from File::sync_all
    // and publication uses MOVEFILE_WRITE_THROUGH.
    let _ = directory.sync_all();
    Ok(())
}

#[cfg(not(windows))]
fn sync_directory(path: &Path) -> Result<(), BackupArchiveError> {
    fs::File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|_| write_failed())
}

fn path_exists(path: &Path) -> Result<bool, BackupArchiveError> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(_) => Err(destination_invalid()),
    }
}

fn path_is_within(candidate: &Path, root: &Path) -> bool {
    #[cfg(windows)]
    {
        let candidate = normalized_path_identity(candidate);
        let root = normalized_path_identity(root);
        candidate == root
            || candidate
                .strip_prefix(&root)
                .is_some_and(|suffix| suffix.starts_with(['\\', '/']))
    }
    #[cfg(not(windows))]
    {
        candidate.starts_with(root)
    }
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

fn normalized_os_identity(value: &OsStr) -> String {
    #[cfg(windows)]
    {
        value.to_string_lossy().to_lowercase()
    }
    #[cfg(not(windows))]
    {
        value.to_string_lossy().into_owned()
    }
}

fn normalized_token_identity(value: &str) -> String {
    #[cfg(windows)]
    {
        value.to_lowercase()
    }
    #[cfg(not(windows))]
    {
        value.to_owned()
    }
}

fn normalized_sql(value: &str) -> String {
    value
        .trim()
        .trim_end_matches(';')
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

fn hex_digest(digest: impl AsRef<[u8]>) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let bytes = digest.as_ref();
    let mut encoded = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}

fn read_u32(file: &mut fs::File) -> Result<u32, BackupArchiveError> {
    let mut bytes = [0_u8; 4];
    file.read_exact(&mut bytes)
        .map_err(|_| verification_failed())?;
    Ok(u32::from_le_bytes(bytes))
}

fn read_u64(file: &mut fs::File) -> Result<u64, BackupArchiveError> {
    let mut bytes = [0_u8; 8];
    file.read_exact(&mut bytes)
        .map_err(|_| verification_failed())?;
    Ok(u64::from_le_bytes(bytes))
}

fn checkpoint(
    fault: &dyn BackupFaultInjector,
    boundary: BackupBoundary,
) -> Result<(), BackupArchiveError> {
    fault.checkpoint(boundary).map_err(|_| write_failed())
}

const fn destination_invalid() -> BackupArchiveError {
    BackupArchiveError::new(MaintenanceErrorCode::BackupDestinationInvalid)
}

const fn destination_exists() -> BackupArchiveError {
    BackupArchiveError::new(MaintenanceErrorCode::BackupDestinationExists)
}

const fn source_unsafe() -> BackupArchiveError {
    BackupArchiveError::new(MaintenanceErrorCode::BackupSourceUnsafe)
}

const fn limit_exceeded() -> BackupArchiveError {
    BackupArchiveError::new(MaintenanceErrorCode::BackupLimitExceeded)
}

const fn snapshot_failed() -> BackupArchiveError {
    BackupArchiveError::new(MaintenanceErrorCode::BackupSnapshotFailed)
}

const fn write_failed() -> BackupArchiveError {
    BackupArchiveError::new(MaintenanceErrorCode::BackupWriteFailed)
}

const fn verification_failed() -> BackupArchiveError {
    BackupArchiveError::new(MaintenanceErrorCode::BackupVerificationFailed)
}

#[cfg(test)]
#[path = "archive_test.rs"]
mod tests;
