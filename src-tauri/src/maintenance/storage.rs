use std::{
    collections::HashSet,
    ffi::OsStr,
    fmt, fs,
    path::{Component, Path, PathBuf},
    process::Command,
};

use uuid::Uuid;

use super::recovery_fs::{ensure_no_reparse_ancestors, require_regular_file};
use crate::{
    app_state::AppPaths,
    domain::{
        MaintenanceErrorCode, MaintenanceErrorDto, StorageCategory, StorageCategoryUsageDto,
        StorageUsageDto,
    },
};

pub const APP_DATA_DIRECTORY_NAME: &str = "dev.textbooklens.desktop";

#[derive(Clone)]
pub struct PreparedAppDataPaths {
    pub root: PathBuf,
    pub books: PathBuf,
    pub cache: PathBuf,
    pub logs: PathBuf,
    pub database: PathBuf,
}

impl fmt::Debug for PreparedAppDataPaths {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PreparedAppDataPaths(<redacted>)")
    }
}

#[derive(Clone)]
pub struct CanonicalAppDataRoot(PathBuf);

impl CanonicalAppDataRoot {
    pub fn as_path(&self) -> &Path {
        &self.0
    }
}

impl fmt::Debug for CanonicalAppDataRoot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("CanonicalAppDataRoot(<redacted>)")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StorageError {
    pub code: MaintenanceErrorCode,
}

impl StorageError {
    const fn new(code: MaintenanceErrorCode) -> Self {
        Self { code }
    }
}

impl fmt::Display for StorageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:?}", self.code)
    }
}

impl std::error::Error for StorageError {}

impl From<StorageError> for MaintenanceErrorDto {
    fn from(error: StorageError) -> Self {
        Self::new(error.code)
    }
}

pub type StorageResult<T> = Result<T, StorageError>;

pub fn prepare_app_data_paths(candidate: &Path) -> StorageResult<PreparedAppDataPaths> {
    validate_untrusted_root_syntax(candidate)?;
    ensure_no_reparse_ancestors(candidate.parent().ok_or_else(root_invalid)?)
        .map_err(|_| root_invalid())?;
    match fs::symlink_metadata(candidate) {
        Ok(metadata) => {
            if !metadata.is_dir() || metadata_is_reparse(&metadata) {
                return Err(root_invalid());
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir(candidate).map_err(|_| root_invalid())?;
        }
        Err(_) => return Err(root_invalid()),
    }
    let root = validate_canonical_app_data_root(candidate)?;
    let books = prepare_owned_directory(&root, "books")?;
    let cache = prepare_owned_directory(&root, "cache")?;
    let logs = prepare_owned_directory(&root, "logs")?;
    let database = root.as_path().join("library.sqlite3");
    validate_database_entry(&root, &database)?;
    Ok(PreparedAppDataPaths {
        root: root.0,
        books,
        cache,
        logs,
        database,
    })
}

pub fn validate_canonical_app_data_root(candidate: &Path) -> StorageResult<CanonicalAppDataRoot> {
    if candidate.as_os_str().is_empty() || !candidate.is_absolute() {
        return Err(root_invalid());
    }
    let metadata = fs::symlink_metadata(candidate).map_err(|_| root_invalid())?;
    if !metadata.is_dir() || metadata_is_reparse(&metadata) {
        return Err(root_invalid());
    }
    ensure_no_reparse_ancestors(candidate).map_err(|_| root_invalid())?;
    let canonical = fs::canonicalize(candidate).map_err(|_| root_invalid())?;
    let canonical_metadata = fs::symlink_metadata(&canonical).map_err(|_| root_invalid())?;
    if !canonical_metadata.is_dir()
        || metadata_is_reparse(&canonical_metadata)
        || canonical.file_name() != Some(OsStr::new(APP_DATA_DIRECTORY_NAME))
        || canonical.parent().is_none()
        || is_forbidden_broad_target(&canonical)
    {
        return Err(root_invalid());
    }
    Ok(CanonicalAppDataRoot(canonical))
}

fn validate_untrusted_root_syntax(candidate: &Path) -> StorageResult<()> {
    if candidate.as_os_str().is_empty()
        || !candidate.is_absolute()
        || candidate.file_name() != Some(OsStr::new(APP_DATA_DIRECTORY_NAME))
        || candidate
            .components()
            .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
    {
        return Err(root_invalid());
    }
    let text = candidate.as_os_str().to_string_lossy();
    if text.contains('*')
        || text.contains('?')
        || text.contains('[')
        || text.contains(']')
        || text.contains("${")
        || text.to_ascii_lowercase().contains("$env:")
        || contains_percent_environment_reference(&text)
    {
        return Err(root_invalid());
    }
    #[cfg(windows)]
    validate_windows_drive_path(candidate)?;
    Ok(())
}

#[cfg(windows)]
fn validate_windows_drive_path(candidate: &Path) -> StorageResult<()> {
    use std::path::Prefix;

    match candidate.components().next() {
        Some(Component::Prefix(prefix)) if matches!(prefix.kind(), Prefix::Disk(_)) => Ok(()),
        _ => Err(root_invalid()),
    }
}

fn contains_percent_environment_reference(value: &str) -> bool {
    let mut indexes = value.match_indices('%').map(|(index, _)| index);
    indexes.next().is_some() && indexes.next().is_some()
}

fn is_forbidden_broad_target(candidate: &Path) -> bool {
    if std::env::current_dir()
        .ok()
        .and_then(|path| fs::canonicalize(path).ok())
        .is_some_and(|workspace| workspace == candidate)
    {
        return true;
    }
    ["USERPROFILE", "HOME"]
        .into_iter()
        .filter_map(std::env::var_os)
        .filter_map(|path| fs::canonicalize(path).ok())
        .any(|home| home == candidate)
}

fn prepare_owned_directory(
    root: &CanonicalAppDataRoot,
    name: &'static str,
) -> StorageResult<PathBuf> {
    let directory = root.as_path().join(name);
    match fs::symlink_metadata(&directory) {
        Ok(metadata) => {
            if !metadata.is_dir() || metadata_is_reparse(&metadata) {
                return Err(root_invalid());
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir(&directory).map_err(|_| root_invalid())?;
        }
        Err(_) => return Err(root_invalid()),
    }
    let canonical = fs::canonicalize(&directory).map_err(|_| root_invalid())?;
    if canonical.parent() != Some(root.as_path())
        || canonical.file_name() != Some(OsStr::new(name))
        || !canonical.starts_with(root.as_path())
    {
        return Err(root_invalid());
    }
    Ok(canonical)
}

fn validate_database_entry(root: &CanonicalAppDataRoot, database: &Path) -> StorageResult<()> {
    if database.parent() != Some(root.as_path())
        || database.file_name() != Some(OsStr::new("library.sqlite3"))
    {
        return Err(root_invalid());
    }
    match fs::symlink_metadata(database) {
        Ok(metadata) if metadata.is_file() && !metadata_is_reparse(&metadata) => {
            require_regular_file(database).map_err(|_| root_invalid())?;
            let canonical = fs::canonicalize(database).map_err(|_| root_invalid())?;
            if canonical.parent() == Some(root.as_path()) {
                Ok(())
            } else {
                Err(root_invalid())
            }
        }
        Ok(_) => Err(root_invalid()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(root_invalid()),
    }
}

#[derive(Clone, Debug)]
pub struct StorageLayout {
    root: CanonicalAppDataRoot,
}

impl StorageLayout {
    pub fn from_app_paths(paths: &AppPaths) -> StorageResult<Self> {
        let root = validate_canonical_app_data_root(&paths.root)?;
        if paths.books != root.as_path().join("books")
            || paths.cache != root.as_path().join("cache")
            || paths.logs != root.as_path().join("logs")
            || paths.database != root.as_path().join("library.sqlite3")
        {
            return Err(root_invalid());
        }
        validate_existing_owned_directory(&root, &paths.books, "books")?;
        validate_existing_owned_directory(&root, &paths.cache, "cache")?;
        validate_existing_owned_directory(&root, &paths.logs, "logs")?;
        validate_database_entry(&root, &paths.database)?;
        Ok(Self { root })
    }

    #[cfg(test)]
    fn from_root(root: &Path) -> StorageResult<Self> {
        Ok(Self {
            root: validate_canonical_app_data_root(root)?,
        })
    }
}

fn validate_existing_owned_directory(
    root: &CanonicalAppDataRoot,
    directory: &Path,
    expected_name: &'static str,
) -> StorageResult<()> {
    let metadata = fs::symlink_metadata(directory).map_err(|_| root_invalid())?;
    if !metadata.is_dir() || metadata_is_reparse(&metadata) {
        return Err(root_invalid());
    }
    let canonical = fs::canonicalize(directory).map_err(|_| root_invalid())?;
    if canonical != directory
        || canonical.parent() != Some(root.as_path())
        || canonical.file_name() != Some(OsStr::new(expected_name))
    {
        return Err(root_invalid());
    }
    Ok(())
}

pub fn collect_storage_usage(layout: &StorageLayout) -> StorageResult<StorageUsageDto> {
    let root = validate_canonical_app_data_root(layout.root.as_path())?;
    if root.as_path() != layout.root.as_path() {
        return Err(root_invalid());
    }
    let mut walker = StorageWalker {
        root,
        seen_directories: HashSet::new(),
        seen_files: HashSet::new(),
        usage: StorageAccumulator::default(),
    };
    let root_path = walker.root.as_path().to_path_buf();
    walker.walk_directory(&root_path)?;
    walker.usage.finish()
}

struct StorageWalker {
    root: CanonicalAppDataRoot,
    seen_directories: HashSet<String>,
    seen_files: HashSet<String>,
    usage: StorageAccumulator,
}

impl StorageWalker {
    fn walk_directory(&mut self, directory: &Path) -> StorageResult<()> {
        let before = stable_metadata(directory)?;
        if !before.is_directory {
            return Err(entry_unsafe());
        }
        let canonical = fs::canonicalize(directory).map_err(|_| scan_failed())?;
        self.require_contained(&canonical)?;
        let relative = canonical
            .strip_prefix(self.root.as_path())
            .map_err(|_| entry_unsafe())?;
        validate_directory_shape(relative)?;
        if !self
            .seen_directories
            .insert(normalized_identity_path(&canonical))
        {
            return Err(entry_unsafe());
        }

        let entries = fs::read_dir(&canonical).map_err(|_| scan_failed())?;
        for entry in entries {
            let entry = entry.map_err(|_| scan_failed())?;
            let path = entry.path();
            if !path.starts_with(self.root.as_path()) {
                return Err(entry_unsafe());
            }
            let metadata = fs::symlink_metadata(&path).map_err(|_| scan_failed())?;
            if metadata_is_reparse(&metadata) {
                return Err(entry_unsafe());
            }
            if metadata.is_dir() {
                self.walk_directory(&path)?;
            } else if metadata.is_file() {
                self.record_file(&path, &metadata)?;
            } else {
                return Err(entry_unsafe());
            }
        }

        let after = stable_metadata(&canonical)?;
        if before != after {
            return Err(scan_failed());
        }
        Ok(())
    }

    fn record_file(&mut self, path: &Path, initial: &fs::Metadata) -> StorageResult<()> {
        if metadata_is_reparse(initial) || !initial.is_file() {
            return Err(entry_unsafe());
        }
        require_regular_file(path).map_err(|_| entry_unsafe())?;
        let canonical = fs::canonicalize(path).map_err(|_| scan_failed())?;
        self.require_contained(&canonical)?;
        let relative = canonical
            .strip_prefix(self.root.as_path())
            .map_err(|_| entry_unsafe())?;
        let category = classify_file(relative)?;
        if !self.seen_files.insert(file_identity(&canonical, initial)?) {
            return Err(entry_unsafe());
        }

        let file = fs::File::open(&canonical).map_err(|_| scan_failed())?;
        let handle_before = file.metadata().map_err(|_| scan_failed())?;
        let path_after = fs::symlink_metadata(&canonical).map_err(|_| scan_failed())?;
        if metadata_is_reparse(&path_after)
            || !same_metadata(initial, &path_after)
            || !same_metadata(&path_after, &handle_before)
        {
            return Err(scan_failed());
        }
        let size = handle_before.len();
        let handle_after = file.metadata().map_err(|_| scan_failed())?;
        if !same_metadata(&handle_before, &handle_after) {
            return Err(scan_failed());
        }
        self.usage.add(category, size)
    }

    fn require_contained(&self, candidate: &Path) -> StorageResult<()> {
        if !candidate.starts_with(self.root.as_path()) {
            return Err(entry_unsafe());
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct StableMetadata {
    is_directory: bool,
    len: u64,
    modified: std::time::SystemTime,
}

fn stable_metadata(path: &Path) -> StorageResult<StableMetadata> {
    let metadata = fs::symlink_metadata(path).map_err(|_| scan_failed())?;
    if metadata_is_reparse(&metadata) || (!metadata.is_dir() && !metadata.is_file()) {
        return Err(entry_unsafe());
    }
    Ok(StableMetadata {
        is_directory: metadata.is_dir(),
        len: metadata.len(),
        modified: metadata.modified().map_err(|_| scan_failed())?,
    })
}

fn same_metadata(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    left.is_dir() == right.is_dir()
        && left.is_file() == right.is_file()
        && left.len() == right.len()
        && left.modified().ok() == right.modified().ok()
}

fn normalized_identity_path(path: &Path) -> String {
    #[cfg(windows)]
    {
        path.as_os_str().to_string_lossy().to_ascii_lowercase()
    }
    #[cfg(not(windows))]
    {
        path.as_os_str().to_string_lossy().into_owned()
    }
}

#[cfg(unix)]
fn file_identity(path: &Path, metadata: &fs::Metadata) -> StorageResult<String> {
    use std::os::unix::fs::MetadataExt;

    Ok(format!("{}:{}", metadata.dev(), metadata.ino()))
}

#[cfg(windows)]
fn file_identity(path: &Path, _metadata: &fs::Metadata) -> StorageResult<String> {
    Ok(normalized_identity_path(path))
}

#[cfg(not(any(unix, windows)))]
fn file_identity(path: &Path, _metadata: &fs::Metadata) -> StorageResult<String> {
    Ok(normalized_identity_path(path))
}

fn validate_directory_shape(relative: &Path) -> StorageResult<()> {
    if relative.as_os_str().is_empty() {
        return Ok(());
    }
    let components = relative.components().collect::<Vec<_>>();
    let first = normal_component(components[0])?;
    match first.to_str() {
        Some("books") => {
            if components.len() == 1 {
                return Ok(());
            }
            let book_id = normal_component(components[1])?
                .to_str()
                .and_then(|value| Uuid::parse_str(value).ok());
            if book_id.is_none() {
                return Err(entry_unsafe());
            }
            if components.len() >= 3 && normal_component(components[2])? != OsStr::new("derived") {
                return Err(entry_unsafe());
            }
            Ok(())
        }
        Some("cache" | "logs" | "maintenance") => Ok(()),
        _ => Err(entry_unsafe()),
    }
}

fn classify_file(relative: &Path) -> StorageResult<StorageCategory> {
    let components = relative.components().collect::<Vec<_>>();
    if components.is_empty() {
        return Err(entry_unsafe());
    }
    let first = normal_component(components[0])?;
    if components.len() == 1 {
        let Some(name) = first.to_str() else {
            return Err(entry_unsafe());
        };
        return if matches!(
            name,
            "library.sqlite3"
                | "library.sqlite3-wal"
                | "library.sqlite3-shm"
                | "library.sqlite3-journal"
        ) {
            Ok(StorageCategory::Database)
        } else {
            Err(entry_unsafe())
        };
    }
    match first.to_str() {
        Some("books") => classify_book_file(&components),
        Some("cache") => {
            if components
                .get(1)
                .and_then(|component| normal_component(*component).ok())
                == Some(OsStr::new("indexing-pages"))
            {
                Ok(StorageCategory::Index)
            } else {
                Ok(StorageCategory::Cache)
            }
        }
        Some("logs") => Ok(StorageCategory::Log),
        Some("maintenance") => Ok(StorageCategory::Cache),
        _ => Err(entry_unsafe()),
    }
}

fn classify_book_file(components: &[Component<'_>]) -> StorageResult<StorageCategory> {
    if components.len() < 3
        || normal_component(components[1])?
            .to_str()
            .and_then(|value| Uuid::parse_str(value).ok())
            .is_none()
    {
        return Err(entry_unsafe());
    }
    let third = normal_component(components[2])?;
    if third == OsStr::new("derived") && components.len() >= 4 {
        return Ok(StorageCategory::Derived);
    }
    if components.len() == 3
        && third
            .to_str()
            .is_some_and(|name| name.starts_with("original."))
    {
        return Ok(StorageCategory::Source);
    }
    Err(entry_unsafe())
}

fn normal_component(component: Component<'_>) -> StorageResult<&OsStr> {
    match component {
        Component::Normal(value) => Ok(value),
        _ => Err(entry_unsafe()),
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct CategoryUsage {
    bytes: u64,
    files: u64,
}

#[derive(Debug, Default)]
struct StorageAccumulator {
    categories: [CategoryUsage; StorageCategory::ALL.len()],
    total_bytes: u64,
    total_files: u64,
}

impl StorageAccumulator {
    fn add(&mut self, category: StorageCategory, bytes: u64) -> StorageResult<()> {
        let category_usage = &mut self.categories[category.index()];
        category_usage.bytes = category_usage
            .bytes
            .checked_add(bytes)
            .ok_or_else(size_overflow)?;
        category_usage.files = category_usage
            .files
            .checked_add(1)
            .ok_or_else(size_overflow)?;
        self.total_bytes = self
            .total_bytes
            .checked_add(bytes)
            .ok_or_else(size_overflow)?;
        self.total_files = self.total_files.checked_add(1).ok_or_else(size_overflow)?;
        Ok(())
    }

    fn finish(self) -> StorageResult<StorageUsageDto> {
        let category_bytes = self
            .categories
            .iter()
            .try_fold(0_u64, |total, usage| total.checked_add(usage.bytes))
            .ok_or_else(size_overflow)?;
        let category_files = self
            .categories
            .iter()
            .try_fold(0_u64, |total, usage| total.checked_add(usage.files))
            .ok_or_else(size_overflow)?;
        if category_bytes != self.total_bytes || category_files != self.total_files {
            return Err(size_overflow());
        }
        Ok(StorageUsageDto {
            total_bytes: self.total_bytes,
            total_file_count: self.total_files,
            categories: StorageCategory::ALL
                .into_iter()
                .map(|category| {
                    let usage = self.categories[category.index()];
                    StorageCategoryUsageDto {
                        category,
                        bytes: usage.bytes,
                        file_count: usage.files,
                    }
                })
                .collect(),
        })
    }
}

pub trait DirectoryLauncher: Send + Sync {
    fn launch(&self, canonical_app_data_root: &Path) -> Result<(), DirectoryLaunchError>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DirectoryLaunchError;

#[derive(Clone, Copy, Debug, Default)]
pub struct WindowsDirectoryLauncher;

impl DirectoryLauncher for WindowsDirectoryLauncher {
    fn launch(&self, canonical_app_data_root: &Path) -> Result<(), DirectoryLaunchError> {
        #[cfg(windows)]
        {
            Command::new("explorer.exe")
                .arg(canonical_app_data_root)
                .spawn()
                .map(|_| ())
                .map_err(|_| DirectoryLaunchError)
        }
        #[cfg(not(windows))]
        {
            let _ = canonical_app_data_root;
            Err(DirectoryLaunchError)
        }
    }
}

pub fn open_canonical_app_data_directory(
    root: &Path,
    launcher: &dyn DirectoryLauncher,
) -> StorageResult<()> {
    let canonical = validate_canonical_app_data_root(root)?;
    launcher
        .launch(canonical.as_path())
        .map_err(|_| StorageError::new(MaintenanceErrorCode::AppDataOpenFailed))
}

#[cfg(windows)]
fn metadata_is_reparse(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn metadata_is_reparse(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

const fn root_invalid() -> StorageError {
    StorageError::new(MaintenanceErrorCode::StorageRootInvalid)
}

const fn entry_unsafe() -> StorageError {
    StorageError::new(MaintenanceErrorCode::StorageEntryUnsafe)
}

const fn scan_failed() -> StorageError {
    StorageError::new(MaintenanceErrorCode::StorageScanFailed)
}

const fn size_overflow() -> StorageError {
    StorageError::new(MaintenanceErrorCode::StorageSizeOverflow)
}

#[cfg(test)]
#[path = "storage_test.rs"]
mod tests;
