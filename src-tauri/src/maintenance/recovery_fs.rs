use std::{
    collections::HashSet,
    ffi::OsStr,
    fmt, fs,
    path::{Path, PathBuf},
};

use super::{
    journal::{metadata_is_reparse, rename_owned_atomic, sync_owned_directory},
    storage::validate_canonical_app_data_root,
};

const MAX_TREE_ENTRIES: usize = 100_000;
const MAX_TREE_DEPTH: usize = 64;
const MAX_TREE_BYTES: u64 = 1 << 40;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OwnedEntryKind {
    File,
    Directory,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct FileIdentity {
    volume: u64,
    index: u64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct EntryWitness {
    kind: OwnedEntryKind,
    identity: FileIdentity,
    len: u64,
    modified: std::time::SystemTime,
    links: u64,
}

#[derive(Clone)]
struct TreeEntry {
    path: PathBuf,
    witness: EntryWitness,
}

pub(crate) struct TreeSnapshot {
    root: PathBuf,
    entries: Vec<TreeEntry>,
}

impl TreeSnapshot {
    pub(crate) fn is_directory_only(&self) -> bool {
        self.entries
            .iter()
            .all(|entry| entry.witness.kind == OwnedEntryKind::Directory)
    }
}

impl fmt::Debug for TreeSnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TreeSnapshot")
            .field("root", &"<redacted>")
            .field("entries", &self.entries.len())
            .finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RecoveryFsError;

impl fmt::Display for RecoveryFsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("unsafe app-owned filesystem state")
    }
}

impl std::error::Error for RecoveryFsError {}

pub(crate) type RecoveryFsResult<T> = Result<T, RecoveryFsError>;

pub(crate) fn require_app_root(root: &Path) -> RecoveryFsResult<PathBuf> {
    let canonical = validate_canonical_app_data_root(root).map_err(|_| RecoveryFsError)?;
    ensure_no_reparse_ancestors(canonical.as_path())?;
    Ok(canonical.as_path().to_path_buf())
}

pub(crate) fn ensure_direct_child_directory(
    parent: &Path,
    name: &str,
) -> RecoveryFsResult<PathBuf> {
    if !safe_component(name) {
        return Err(RecoveryFsError);
    }
    require_directory(parent)?;
    let path = parent.join(name);
    match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.is_dir() && !metadata_is_reparse(&metadata) => {}
        Ok(_) => return Err(RecoveryFsError),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir(&path).map_err(|_| RecoveryFsError)?;
            sync(parent)?;
        }
        Err(_) => return Err(RecoveryFsError),
    }
    require_exact_child_directory(parent, &path)?;
    Ok(path)
}

pub(crate) fn require_exact_child_directory(parent: &Path, child: &Path) -> RecoveryFsResult<()> {
    if child.parent() != Some(parent) || child.file_name().is_none() {
        return Err(RecoveryFsError);
    }
    require_directory(parent)?;
    require_directory(child)?;
    let canonical_parent = fs::canonicalize(parent).map_err(|_| RecoveryFsError)?;
    let canonical_child = fs::canonicalize(child).map_err(|_| RecoveryFsError)?;
    if canonical_parent != parent
        || canonical_child != child
        || canonical_child.parent() != Some(canonical_parent.as_path())
    {
        return Err(RecoveryFsError);
    }
    Ok(())
}

pub(crate) fn require_directory(path: &Path) -> RecoveryFsResult<()> {
    let witness = witness(path)?;
    if witness.kind != OwnedEntryKind::Directory {
        return Err(RecoveryFsError);
    }
    Ok(())
}

pub(crate) fn require_regular_file(path: &Path) -> RecoveryFsResult<()> {
    let witness = witness(path)?;
    if witness.kind != OwnedEntryKind::File || witness.links != 1 {
        return Err(RecoveryFsError);
    }
    Ok(())
}

pub(crate) fn require_directory_children(
    parent: &Path,
    allowed: &[&str],
    require_all: bool,
) -> RecoveryFsResult<Vec<String>> {
    require_directory(parent)?;
    if allowed.iter().any(|name| !safe_component(name)) {
        return Err(RecoveryFsError);
    }
    let allowed = allowed.iter().copied().collect::<HashSet<_>>();
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for entry in fs::read_dir(parent).map_err(|_| RecoveryFsError)? {
        if result.len() >= MAX_TREE_ENTRIES {
            return Err(RecoveryFsError);
        }
        let entry = entry.map_err(|_| RecoveryFsError)?;
        let path = entry.path();
        if path.parent() != Some(parent) {
            return Err(RecoveryFsError);
        }
        let name = entry
            .file_name()
            .to_str()
            .filter(|name| allowed.contains(*name))
            .ok_or(RecoveryFsError)?
            .to_owned();
        let normalized = normalized_name(OsStr::new(&name));
        if !seen.insert(normalized) {
            return Err(RecoveryFsError);
        }
        require_exact_child_directory(parent, &path)?;
        result.push(name);
    }
    if require_all && result.len() != allowed.len() {
        return Err(RecoveryFsError);
    }
    result.sort();
    Ok(result)
}

pub(crate) fn path_exists(path: &Path) -> RecoveryFsResult<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(_) => Err(RecoveryFsError),
    }
}

pub(crate) fn rename_checked(
    app_root: &Path,
    source: &Path,
    destination: &Path,
    kind: OwnedEntryKind,
) -> RecoveryFsResult<()> {
    require_contained(app_root, source)?;
    require_contained(app_root, destination)?;
    if source == destination
        || destination.parent().is_none()
        || path_exists(destination)?
        || !path_exists(source)?
    {
        return Err(RecoveryFsError);
    }
    let before = witness(source)?;
    if before.kind != kind || (kind == OwnedEntryKind::File && before.links != 1) {
        return Err(RecoveryFsError);
    }
    let destination_parent = destination.parent().ok_or(RecoveryFsError)?;
    require_directory(destination_parent)?;
    let parent_witness = witness(destination_parent)?;
    if before.identity.volume != parent_witness.identity.volume {
        return Err(RecoveryFsError);
    }
    rename_owned_atomic(source, destination).map_err(|_| RecoveryFsError)?;
    sync(source.parent().ok_or(RecoveryFsError)?)?;
    if source.parent() != Some(destination_parent) {
        sync(destination_parent)?;
    }
    let after = witness(destination)?;
    if after != before || path_exists(source)? {
        return Err(RecoveryFsError);
    }
    Ok(())
}

pub(crate) fn capture_tree(
    app_root: &Path,
    target: &Path,
    kind: OwnedEntryKind,
) -> RecoveryFsResult<TreeSnapshot> {
    require_contained(app_root, target)?;
    let mut snapshot = TreeSnapshot {
        root: target.to_path_buf(),
        entries: Vec::new(),
    };
    let mut identities = HashSet::new();
    let mut bytes = 0_u64;
    capture_node(
        app_root,
        target,
        kind,
        0,
        &mut snapshot.entries,
        &mut identities,
        &mut bytes,
    )?;
    Ok(snapshot)
}

fn capture_node(
    app_root: &Path,
    path: &Path,
    expected_kind: OwnedEntryKind,
    depth: usize,
    entries: &mut Vec<TreeEntry>,
    identities: &mut HashSet<FileIdentity>,
    bytes: &mut u64,
) -> RecoveryFsResult<()> {
    if depth > MAX_TREE_DEPTH || entries.len() >= MAX_TREE_ENTRIES {
        return Err(RecoveryFsError);
    }
    require_contained(app_root, path)?;
    let before = witness(path)?;
    if before.kind != expected_kind || !identities.insert(before.identity) {
        return Err(RecoveryFsError);
    }
    if before.kind == OwnedEntryKind::File {
        if before.links != 1 {
            return Err(RecoveryFsError);
        }
        *bytes = bytes
            .checked_add(before.len)
            .filter(|total| *total <= MAX_TREE_BYTES)
            .ok_or(RecoveryFsError)?;
    } else {
        let canonical = fs::canonicalize(path).map_err(|_| RecoveryFsError)?;
        if canonical != path {
            return Err(RecoveryFsError);
        }
        let mut children = Vec::new();
        for entry in fs::read_dir(path).map_err(|_| RecoveryFsError)? {
            if children.len() >= MAX_TREE_ENTRIES {
                return Err(RecoveryFsError);
            }
            children.push(entry.map_err(|_| RecoveryFsError)?);
        }
        children.sort_by_key(|entry| normalized_name(&entry.file_name()));
        for child in children {
            let child_path = child.path();
            if child_path.parent() != Some(path) {
                return Err(RecoveryFsError);
            }
            let metadata = fs::symlink_metadata(&child_path).map_err(|_| RecoveryFsError)?;
            let child_kind = if metadata_is_reparse(&metadata) {
                return Err(RecoveryFsError);
            } else if metadata.is_dir() {
                OwnedEntryKind::Directory
            } else if metadata.is_file() {
                OwnedEntryKind::File
            } else {
                return Err(RecoveryFsError);
            };
            capture_node(
                app_root,
                &child_path,
                child_kind,
                depth + 1,
                entries,
                identities,
                bytes,
            )?;
        }
        if witness(path)? != before {
            return Err(RecoveryFsError);
        }
    }
    entries.push(TreeEntry {
        path: path.to_path_buf(),
        witness: before,
    });
    Ok(())
}

pub(crate) fn remove_captured_tree(
    app_root: &Path,
    snapshot: &TreeSnapshot,
) -> RecoveryFsResult<()> {
    require_contained(app_root, &snapshot.root)?;
    for entry in &snapshot.entries {
        require_contained(app_root, &entry.path)?;
        if witness(&entry.path)? != entry.witness {
            return Err(RecoveryFsError);
        }
    }
    for entry in &snapshot.entries {
        let current = witness(&entry.path)?;
        let matches = match entry.witness.kind {
            OwnedEntryKind::File => current == entry.witness,
            OwnedEntryKind::Directory => {
                current.kind == entry.witness.kind
                    && current.identity == entry.witness.identity
                    && current.links == entry.witness.links
            }
        };
        if !matches {
            return Err(RecoveryFsError);
        }
        match entry.witness.kind {
            OwnedEntryKind::File => fs::remove_file(&entry.path),
            OwnedEntryKind::Directory => fs::remove_dir(&entry.path),
        }
        .map_err(|_| RecoveryFsError)?;
        sync(entry.path.parent().ok_or(RecoveryFsError)?)?;
    }
    Ok(())
}

pub(crate) fn remove_tree(
    app_root: &Path,
    target: &Path,
    kind: OwnedEntryKind,
) -> RecoveryFsResult<()> {
    let snapshot = capture_tree(app_root, target, kind)?;
    remove_captured_tree(app_root, &snapshot)
}

pub(crate) fn sync(path: &Path) -> RecoveryFsResult<()> {
    sync_owned_directory(path).map_err(|_| RecoveryFsError)
}

pub(crate) fn require_contained(app_root: &Path, candidate: &Path) -> RecoveryFsResult<()> {
    if candidate == app_root || !path_is_within(candidate, app_root) {
        return Err(RecoveryFsError);
    }
    Ok(())
}

fn witness(path: &Path) -> RecoveryFsResult<EntryWitness> {
    let metadata = fs::symlink_metadata(path).map_err(|_| RecoveryFsError)?;
    if metadata_is_reparse(&metadata) {
        return Err(RecoveryFsError);
    }
    let kind = if metadata.is_file() {
        OwnedEntryKind::File
    } else if metadata.is_dir() {
        OwnedEntryKind::Directory
    } else {
        return Err(RecoveryFsError);
    };
    let canonical = fs::canonicalize(path).map_err(|_| RecoveryFsError)?;
    if canonical != path {
        return Err(RecoveryFsError);
    }
    let info = file_information(path, kind)?;
    Ok(EntryWitness {
        kind,
        identity: info.identity,
        len: metadata.len(),
        modified: metadata.modified().map_err(|_| RecoveryFsError)?,
        links: info.links,
    })
}

#[derive(Clone, Copy)]
struct FileInformation {
    identity: FileIdentity,
    links: u64,
}

#[cfg(unix)]
fn file_information(path: &Path, _kind: OwnedEntryKind) -> RecoveryFsResult<FileInformation> {
    use std::os::unix::fs::MetadataExt;
    let metadata = fs::symlink_metadata(path).map_err(|_| RecoveryFsError)?;
    Ok(FileInformation {
        identity: FileIdentity {
            volume: metadata.dev(),
            index: metadata.ino(),
        },
        links: metadata.nlink(),
    })
}

#[cfg(windows)]
fn file_information(path: &Path, kind: OwnedEntryKind) -> RecoveryFsResult<FileInformation> {
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
    let flags = FILE_FLAG_OPEN_REPARSE_POINT
        | if kind == OwnedEntryKind::Directory {
            FILE_FLAG_BACKUP_SEMANTICS
        } else {
            0
        };
    // SAFETY: the path is a live NUL-terminated UTF-16 buffer.
    let handle = unsafe {
        create_file_w(
            wide.as_ptr(),
            FILE_READ_ATTRIBUTES,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null_mut(),
            OPEN_EXISTING,
            flags,
            std::ptr::null_mut(),
        )
    };
    if handle as isize == -1 {
        return Err(RecoveryFsError);
    }
    // SAFETY: the integer-only C representation admits zero initialization.
    let mut information: WindowsByHandleFileInformation = unsafe { std::mem::zeroed() };
    // SAFETY: the handle and writable output are valid during this call.
    let succeeded = unsafe { get_file_information_by_handle(handle, &mut information) } != 0;
    // SAFETY: the handle was returned by CreateFileW and is closed once.
    let _ = unsafe { close_handle(handle) };
    if !succeeded {
        return Err(RecoveryFsError);
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
fn file_information(_path: &Path, _kind: OwnedEntryKind) -> RecoveryFsResult<FileInformation> {
    Err(RecoveryFsError)
}

fn ensure_no_reparse_ancestors(path: &Path) -> RecoveryFsResult<()> {
    for ancestor in path.ancestors() {
        if ancestor.as_os_str().is_empty() {
            continue;
        }
        let metadata = fs::symlink_metadata(ancestor).map_err(|_| RecoveryFsError)?;
        if metadata_is_reparse(&metadata) || !metadata.is_dir() {
            return Err(RecoveryFsError);
        }
    }
    Ok(())
}

fn path_is_within(candidate: &Path, root: &Path) -> bool {
    #[cfg(windows)]
    {
        let candidate = candidate.as_os_str().to_string_lossy().to_lowercase();
        let root = root.as_os_str().to_string_lossy().to_lowercase();
        candidate
            .strip_prefix(&root)
            .is_some_and(|suffix| suffix.starts_with(['\\', '/']))
    }
    #[cfg(not(windows))]
    {
        candidate.starts_with(root) && candidate != root
    }
}

fn safe_component(value: &str) -> bool {
    !value.is_empty()
        && value != "."
        && value != ".."
        && !value.ends_with(['.', ' '])
        && !value.contains(['\\', '/', ':', '\0'])
        && !value
            .chars()
            .any(|character| character.is_ascii_control() || "<>\"|?*[]".contains(character))
        && !is_windows_device_name(value)
}

fn is_windows_device_name(value: &str) -> bool {
    let stem = value
        .split_once('.')
        .map_or(value, |(stem, _)| stem)
        .to_ascii_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || stem
            .strip_prefix("COM")
            .or_else(|| stem.strip_prefix("LPT"))
            .is_some_and(|suffix| suffix.len() == 1 && matches!(suffix.as_bytes()[0], b'1'..=b'9'))
}

fn normalized_name(value: &OsStr) -> String {
    #[cfg(windows)]
    {
        value.to_string_lossy().to_lowercase()
    }
    #[cfg(not(windows))]
    {
        value.to_string_lossy().into_owned()
    }
}
