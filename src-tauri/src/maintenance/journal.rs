use std::{
    collections::HashSet,
    ffi::OsStr,
    fmt, fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

use serde::{
    Deserialize, Deserializer, Serialize,
    de::{Error as _, SeqAccess, Visitor},
};
use uuid::Uuid;

use crate::{
    app_state::AppPaths,
    errors::{AppError, AppErrorCode},
    maintenance::storage::StorageLayout,
};

const DELETE_INTENT_FORMAT: &str = "textbooklens.delete-book-intent";
const DELETE_COMMIT_FORMAT: &str = "textbooklens.delete-book-commit";
const DELETE_JOURNAL_VERSION: u32 = 1;
const MAX_RELATIVE_TOKEN_BYTES: usize = 512;
pub const MAX_DELETE_JOURNAL_ENTRIES: usize = 10_001;
const MAX_DELETE_JOURNAL_BYTES: u64 = 4 * 1024 * 1024;
const MAX_PENDING_DELETE_JOURNALS: usize = 64;
const MAX_DELETE_DIRECTORY_ENTRIES: usize = MAX_PENDING_DELETE_JOURNALS * 4;

const MAINTENANCE_DIRECTORY: &str = "maintenance";
const DELETE_DIRECTORY: &str = "delete-book";
const INTENTS_DIRECTORY: &str = "intents";
const TRASH_DIRECTORY: &str = "trash";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JournalErrorCode {
    UnsafeLayout,
    InvalidIntent,
    LimitExceeded,
    Io,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct JournalError {
    pub code: JournalErrorCode,
}

impl JournalError {
    const fn new(code: JournalErrorCode) -> Self {
        Self { code }
    }

    pub fn into_app_error(self) -> AppError {
        match self.code {
            JournalErrorCode::UnsafeLayout | JournalErrorCode::InvalidIntent => {
                AppError::new(AppErrorCode::InvalidInput)
            }
            JournalErrorCode::LimitExceeded => AppError::new(AppErrorCode::RequestConflict),
            JournalErrorCode::Io => AppError::new(AppErrorCode::LocalIoError),
        }
    }
}

impl fmt::Debug for JournalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("JournalError")
            .field("code", &self.code)
            .finish()
    }
}

impl fmt::Display for JournalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:?}", self.code)
    }
}

impl std::error::Error for JournalError {}

pub type JournalResult<T> = Result<T, JournalError>;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RelativePathToken(String);

impl RelativePathToken {
    pub fn new(value: String) -> JournalResult<Self> {
        validate_relative_token(&value)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn to_path(&self, root: &Path) -> PathBuf {
        self.0
            .split('/')
            .fold(root.to_path_buf(), |path, part| path.join(part))
    }
}

impl fmt::Debug for RelativePathToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("RelativePathToken(<redacted>)")
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeleteJournalEntry {
    source: RelativePathToken,
    trash: RelativePathToken,
}

impl DeleteJournalEntry {
    pub fn source(&self) -> &RelativePathToken {
        &self.source
    }

    pub fn trash(&self) -> &RelativePathToken {
        &self.trash
    }
}

impl fmt::Debug for DeleteJournalEntry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("DeleteJournalEntry(<redacted>)")
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeleteJournal {
    format: String,
    version: u32,
    journal_id: Uuid,
    book_id: Uuid,
    #[serde(deserialize_with = "deserialize_bounded_entries")]
    entries: Vec<DeleteJournalEntry>,
}

fn deserialize_bounded_entries<'de, D>(deserializer: D) -> Result<Vec<DeleteJournalEntry>, D::Error>
where
    D: Deserializer<'de>,
{
    struct BoundedEntries;

    impl<'de> Visitor<'de> for BoundedEntries {
        type Value = Vec<DeleteJournalEntry>;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a bounded delete journal entry list")
        }

        fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
        where
            A: SeqAccess<'de>,
        {
            if sequence
                .size_hint()
                .is_some_and(|size| size > MAX_DELETE_JOURNAL_ENTRIES)
            {
                return Err(A::Error::custom("delete journal entry limit exceeded"));
            }
            let mut entries = Vec::with_capacity(
                sequence
                    .size_hint()
                    .unwrap_or_default()
                    .min(MAX_DELETE_JOURNAL_ENTRIES),
            );
            while let Some(entry) = sequence.next_element()? {
                if entries.len() >= MAX_DELETE_JOURNAL_ENTRIES {
                    return Err(A::Error::custom("delete journal entry limit exceeded"));
                }
                entries.push(entry);
            }
            Ok(entries)
        }
    }

    deserializer.deserialize_seq(BoundedEntries)
}

impl DeleteJournal {
    pub fn from_sources(
        journal_id: Uuid,
        book_id: Uuid,
        sources: Vec<RelativePathToken>,
    ) -> JournalResult<Self> {
        let entries = sources
            .into_iter()
            .enumerate()
            .map(|(index, source)| {
                Ok(DeleteJournalEntry {
                    source,
                    trash: RelativePathToken::new(format!(
                        "cache/{MAINTENANCE_DIRECTORY}/{DELETE_DIRECTORY}/{TRASH_DIRECTORY}/{journal_id}/{index}"
                    ))?,
                })
            })
            .collect::<JournalResult<Vec<_>>>()?;
        let journal = Self {
            format: DELETE_INTENT_FORMAT.to_owned(),
            version: DELETE_JOURNAL_VERSION,
            journal_id,
            book_id,
            entries,
        };
        journal.validate()?;
        Ok(journal)
    }

    pub fn journal_id(&self) -> Uuid {
        self.journal_id
    }

    pub fn book_id(&self) -> Uuid {
        self.book_id
    }

    pub fn entries(&self) -> &[DeleteJournalEntry] {
        &self.entries
    }

    fn validate(&self) -> JournalResult<()> {
        if self.format != DELETE_INTENT_FORMAT
            || self.version != DELETE_JOURNAL_VERSION
            || self.entries.len() > MAX_DELETE_JOURNAL_ENTRIES
        {
            return Err(invalid_intent());
        }

        let expected_book_source = format!("books/{}", self.book_id);
        let expected_trash_prefix = format!(
            "cache/{MAINTENANCE_DIRECTORY}/{DELETE_DIRECTORY}/{TRASH_DIRECTORY}/{}/",
            self.journal_id
        );
        let mut source_identities = HashSet::with_capacity(self.entries.len());
        let mut trash_identities = HashSet::with_capacity(self.entries.len());
        let mut saw_book_directory = false;
        let mut previous_page = None;

        for (index, entry) in self.entries.iter().enumerate() {
            validate_relative_token(entry.source.as_str())?;
            validate_relative_token(entry.trash.as_str())?;
            let expected_trash = format!("{expected_trash_prefix}{index}");
            if entry.trash.as_str() != expected_trash {
                return Err(invalid_intent());
            }
            if !source_identities.insert(normalized_token_identity(entry.source.as_str()))
                || !trash_identities.insert(normalized_token_identity(entry.trash.as_str()))
            {
                return Err(invalid_intent());
            }

            if entry.source.as_str() == expected_book_source {
                if saw_book_directory || index != 0 {
                    return Err(invalid_intent());
                }
                saw_book_directory = true;
                continue;
            }

            let Some(page_id) = parse_page_scratch_token(entry.source.as_str()) else {
                return Err(invalid_intent());
            };
            if previous_page.is_some_and(|previous| previous >= page_id) {
                return Err(invalid_intent());
            }
            previous_page = Some(page_id);
        }
        Ok(())
    }
}

impl fmt::Debug for DeleteJournal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DeleteJournal")
            .field("version", &self.version)
            .field("entry_count", &self.entries.len())
            .field("identifiers", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DeleteCommitMarker {
    format: String,
    version: u32,
    journal_id: Uuid,
}

impl DeleteCommitMarker {
    fn new(journal_id: Uuid) -> Self {
        Self {
            format: DELETE_COMMIT_FORMAT.to_owned(),
            version: DELETE_JOURNAL_VERSION,
            journal_id,
        }
    }

    fn validate(&self, expected_journal_id: Uuid) -> JournalResult<()> {
        if self.format != DELETE_COMMIT_FORMAT
            || self.version != DELETE_JOURNAL_VERSION
            || self.journal_id != expected_journal_id
        {
            return Err(invalid_intent());
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum JournalBoundary {
    IntentTempCreated,
    IntentTempWritten,
    IntentTempSynced,
    IntentRenamed,
    IntentParentSynced,
    CommitTempCreated,
    CommitTempWritten,
    CommitTempSynced,
    CommitRenamed,
    CommitParentSynced,
    CommitRemoved,
    CommitRemovalSynced,
    IntentRemoved,
    IntentRemovalSynced,
}

impl JournalBoundary {
    pub const ALL: [Self; 14] = [
        Self::IntentTempCreated,
        Self::IntentTempWritten,
        Self::IntentTempSynced,
        Self::IntentRenamed,
        Self::IntentParentSynced,
        Self::CommitTempCreated,
        Self::CommitTempWritten,
        Self::CommitTempSynced,
        Self::CommitRenamed,
        Self::CommitParentSynced,
        Self::CommitRemoved,
        Self::CommitRemovalSynced,
        Self::IntentRemoved,
        Self::IntentRemovalSynced,
    ];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct JournalInjectedCrash {
    boundary: JournalBoundary,
}

impl JournalInjectedCrash {
    pub fn at(boundary: JournalBoundary) -> Self {
        Self { boundary }
    }

    pub fn boundary(self) -> JournalBoundary {
        self.boundary
    }
}

pub trait JournalFaultInjector: Send + Sync {
    fn checkpoint(&self, boundary: JournalBoundary) -> Result<(), JournalInjectedCrash>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct NoJournalFault;

impl JournalFaultInjector for NoJournalFault {
    fn checkpoint(&self, _boundary: JournalBoundary) -> Result<(), JournalInjectedCrash> {
        Ok(())
    }
}

pub enum JournalOperationError {
    Journal(JournalError),
    Injected(JournalInjectedCrash),
}

impl fmt::Debug for JournalOperationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Journal(error) => formatter.debug_tuple("Journal").field(error).finish(),
            Self::Injected(crash) => formatter
                .debug_struct("Injected")
                .field("boundary", &crash.boundary)
                .finish(),
        }
    }
}

impl From<JournalError> for JournalOperationError {
    fn from(error: JournalError) -> Self {
        Self::Journal(error)
    }
}

#[derive(Clone)]
pub struct JournalStore {
    root: PathBuf,
    intents: PathBuf,
    trash: PathBuf,
}

impl JournalStore {
    pub fn open(paths: &AppPaths) -> JournalResult<Self> {
        StorageLayout::from_app_paths(paths).map_err(|_| unsafe_layout())?;
        let cache = canonical_owned_directory(&paths.root, &paths.cache, "cache")?;
        let maintenance = ensure_owned_directory(&cache, MAINTENANCE_DIRECTORY)?;
        let delete = ensure_owned_directory(&maintenance, DELETE_DIRECTORY)?;
        let intents = ensure_owned_directory(&delete, INTENTS_DIRECTORY)?;
        let trash = ensure_owned_directory(&delete, TRASH_DIRECTORY)?;
        Ok(Self {
            root: paths.root.clone(),
            intents,
            trash,
        })
    }

    pub fn resolve(&self, token: &RelativePathToken) -> JournalResult<PathBuf> {
        validate_relative_token(token.as_str())?;
        Ok(token.to_path(&self.root))
    }

    pub fn trash_directory(&self) -> &Path {
        &self.trash
    }

    pub(crate) fn root_directory(&self) -> &Path {
        &self.root
    }

    pub fn journal_trash_root(&self, journal_id: Uuid) -> PathBuf {
        self.trash.join(journal_id.to_string())
    }

    pub fn create_journal_trash_root(&self, journal_id: Uuid) -> JournalResult<PathBuf> {
        let path = self.journal_trash_root(journal_id);
        require_missing(&path)?;
        fs::create_dir(&path).map_err(|_| io_error())?;
        let canonical = fs::canonicalize(&path).map_err(|_| io_error())?;
        if canonical != path || canonical.parent() != Some(self.trash.as_path()) {
            return Err(unsafe_layout());
        }
        sync_directory(&self.trash)?;
        Ok(path)
    }

    pub fn write_intent(
        &self,
        journal: &DeleteJournal,
        fault: &dyn JournalFaultInjector,
    ) -> Result<(), JournalOperationError> {
        journal.validate()?;
        let bytes = serde_json::to_vec(journal).map_err(|_| invalid_intent())?;
        if bytes.len() as u64 > MAX_DELETE_JOURNAL_BYTES {
            return Err(limit_exceeded().into());
        }
        let id = journal.journal_id();
        self.write_atomic_json(
            &self.intents.join(format!("{id}.intent.tmp")),
            &self.intents.join(format!("{id}.json")),
            &bytes,
            [
                JournalBoundary::IntentTempCreated,
                JournalBoundary::IntentTempWritten,
                JournalBoundary::IntentTempSynced,
                JournalBoundary::IntentRenamed,
                JournalBoundary::IntentParentSynced,
            ],
            fault,
        )
    }

    pub fn write_commit_marker(
        &self,
        journal_id: Uuid,
        fault: &dyn JournalFaultInjector,
    ) -> Result<(), JournalOperationError> {
        let bytes = serde_json::to_vec(&DeleteCommitMarker::new(journal_id))
            .map_err(|_| invalid_intent())?;
        self.write_atomic_json(
            &self.intents.join(format!("{journal_id}.commit.tmp")),
            &self.intents.join(format!("{journal_id}.committed")),
            &bytes,
            [
                JournalBoundary::CommitTempCreated,
                JournalBoundary::CommitTempWritten,
                JournalBoundary::CommitTempSynced,
                JournalBoundary::CommitRenamed,
                JournalBoundary::CommitParentSynced,
            ],
            fault,
        )
    }

    fn write_atomic_json(
        &self,
        temporary: &Path,
        final_path: &Path,
        bytes: &[u8],
        boundaries: [JournalBoundary; 5],
        fault: &dyn JournalFaultInjector,
    ) -> Result<(), JournalOperationError> {
        require_missing(temporary)?;
        require_missing(final_path)?;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(temporary)
            .map_err(|_| io_error())?;
        fault
            .checkpoint(boundaries[0])
            .map_err(JournalOperationError::Injected)?;
        file.write_all(bytes).map_err(|_| io_error())?;
        file.flush().map_err(|_| io_error())?;
        fault
            .checkpoint(boundaries[1])
            .map_err(JournalOperationError::Injected)?;
        file.sync_all().map_err(|_| io_error())?;
        fault
            .checkpoint(boundaries[2])
            .map_err(JournalOperationError::Injected)?;
        drop(file);
        rename_owned_atomic(temporary, final_path)?;
        fault
            .checkpoint(boundaries[3])
            .map_err(JournalOperationError::Injected)?;
        sync_directory(&self.intents)?;
        fault
            .checkpoint(boundaries[4])
            .map_err(JournalOperationError::Injected)?;
        Ok(())
    }

    pub fn list_intents(&self) -> JournalResult<Vec<DeleteJournal>> {
        let mut journals = Vec::new();
        let mut intent_ids = HashSet::new();
        let mut committed_ids = HashSet::new();
        let mut directory_entry_count = 0_usize;
        for entry in fs::read_dir(&self.intents).map_err(|_| io_error())? {
            directory_entry_count = directory_entry_count
                .checked_add(1)
                .filter(|count| *count <= MAX_DELETE_DIRECTORY_ENTRIES)
                .ok_or_else(limit_exceeded)?;
            let entry = entry.map_err(|_| io_error())?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                return Err(invalid_intent());
            };
            if is_known_temporary_name(name) {
                continue;
            }
            if let Some(id) = name
                .strip_suffix(".committed")
                .and_then(parse_canonical_uuid)
            {
                if committed_ids.len() >= MAX_PENDING_DELETE_JOURNALS || !committed_ids.insert(id) {
                    return Err(limit_exceeded());
                }
                self.validate_commit_marker_if_present(id)?;
                continue;
            }
            let Some(id) = name.strip_suffix(".json").and_then(parse_canonical_uuid) else {
                return Err(invalid_intent());
            };
            if journals.len() >= MAX_PENDING_DELETE_JOURNALS || !intent_ids.insert(id) {
                return Err(limit_exceeded());
            }
            let journal = self.read_intent(id)?;
            if journal.journal_id() != id {
                return Err(invalid_intent());
            }
            self.validate_commit_marker_if_present(id)?;
            journals.push(journal);
        }
        if committed_ids
            .iter()
            .any(|journal_id| !intent_ids.contains(journal_id))
        {
            return Err(invalid_intent());
        }
        journals.sort_unstable_by_key(DeleteJournal::journal_id);
        Ok(journals)
    }

    pub fn read_intent(&self, journal_id: Uuid) -> JournalResult<DeleteJournal> {
        let bytes = read_bounded_regular_file(
            &self.intents,
            &self.intents.join(format!("{journal_id}.json")),
            MAX_DELETE_JOURNAL_BYTES,
        )?;
        let journal: DeleteJournal =
            serde_json::from_slice(&bytes).map_err(|_| invalid_intent())?;
        journal.validate()?;
        if journal.journal_id() != journal_id {
            return Err(invalid_intent());
        }
        Ok(journal)
    }

    fn validate_commit_marker_if_present(&self, journal_id: Uuid) -> JournalResult<()> {
        let marker_path = self.intents.join(format!("{journal_id}.committed"));
        match fs::symlink_metadata(&marker_path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err(io_error()),
            Ok(_) => {
                let bytes = read_bounded_regular_file(&self.intents, &marker_path, 1024)?;
                let marker: DeleteCommitMarker =
                    serde_json::from_slice(&bytes).map_err(|_| invalid_intent())?;
                marker.validate(journal_id)
            }
        }
    }

    pub fn remove_journal_files(
        &self,
        journal_id: Uuid,
        fault: &dyn JournalFaultInjector,
    ) -> Result<(), JournalOperationError> {
        let marker = self.intents.join(format!("{journal_id}.committed"));
        remove_regular_file_if_present(&self.intents, &marker)?;
        fault
            .checkpoint(JournalBoundary::CommitRemoved)
            .map_err(JournalOperationError::Injected)?;
        sync_directory(&self.intents)?;
        fault
            .checkpoint(JournalBoundary::CommitRemovalSynced)
            .map_err(JournalOperationError::Injected)?;

        let intent = self.intents.join(format!("{journal_id}.json"));
        remove_regular_file_if_present(&self.intents, &intent)?;
        fault
            .checkpoint(JournalBoundary::IntentRemoved)
            .map_err(JournalOperationError::Injected)?;
        sync_directory(&self.intents)?;
        fault
            .checkpoint(JournalBoundary::IntentRemovalSynced)
            .map_err(JournalOperationError::Injected)?;
        Ok(())
    }

    pub fn cleanup_stale_temporary_files(&self) -> JournalResult<()> {
        let mut removed = false;
        let mut directory_entry_count = 0_usize;
        for entry in fs::read_dir(&self.intents).map_err(|_| io_error())? {
            directory_entry_count = directory_entry_count
                .checked_add(1)
                .filter(|count| *count <= MAX_DELETE_DIRECTORY_ENTRIES)
                .ok_or_else(limit_exceeded)?;
            let entry = entry.map_err(|_| io_error())?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                return Err(invalid_intent());
            };
            if !is_known_temporary_name(name) {
                continue;
            }
            remove_regular_file_if_present(&self.intents, &entry.path())?;
            removed = true;
        }
        if removed {
            sync_directory(&self.intents)?;
        }
        Ok(())
    }
}

impl fmt::Debug for JournalStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("JournalStore(<redacted>)")
    }
}

fn validate_relative_token(value: &str) -> JournalResult<()> {
    if value.is_empty()
        || value.len() > MAX_RELATIVE_TOKEN_BYTES
        || !value.is_ascii()
        || value != value.to_ascii_lowercase()
        || value.starts_with('/')
        || value.ends_with('/')
        || value.contains(['\\', ':', '\0'])
    {
        return Err(invalid_intent());
    }
    let mut component_count = 0usize;
    for component in value.split('/') {
        component_count = component_count.checked_add(1).ok_or_else(limit_exceeded)?;
        if component.is_empty()
            || matches!(component, "." | "..")
            || component.ends_with(['.', ' '])
            || !component.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"-_.".contains(&byte)
            })
            || is_windows_device_name(component)
        {
            return Err(invalid_intent());
        }
    }
    if component_count == 0 || !matches!(value.split('/').next(), Some("books" | "cache")) {
        return Err(invalid_intent());
    }
    Ok(())
}

fn parse_page_scratch_token(value: &str) -> Option<Uuid> {
    let mut parts = value.split('/');
    if parts.next()? != "cache" || parts.next()? != "indexing-pages" {
        return None;
    }
    let page = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    parse_canonical_uuid(page)
}

fn parse_canonical_uuid(value: &str) -> Option<Uuid> {
    let id = Uuid::parse_str(value).ok()?;
    (id.to_string() == value).then_some(id)
}

fn normalized_token_identity(value: &str) -> String {
    value.replace('\\', "/").to_ascii_lowercase()
}

fn is_windows_device_name(component: &str) -> bool {
    let stem = component.split('.').next().unwrap_or(component);
    matches!(stem, "con" | "prn" | "aux" | "nul")
        || stem
            .strip_prefix("com")
            .or_else(|| stem.strip_prefix("lpt"))
            .is_some_and(|suffix| {
                matches!(suffix, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
            })
}

fn is_known_temporary_name(name: &str) -> bool {
    name.strip_suffix(".intent.tmp")
        .or_else(|| name.strip_suffix(".commit.tmp"))
        .and_then(parse_canonical_uuid)
        .is_some()
}

fn canonical_owned_directory(root: &Path, candidate: &Path, name: &str) -> JournalResult<PathBuf> {
    let metadata = fs::symlink_metadata(candidate).map_err(|_| unsafe_layout())?;
    if !metadata.is_dir() || metadata_is_reparse(&metadata) {
        return Err(unsafe_layout());
    }
    let canonical = fs::canonicalize(candidate).map_err(|_| unsafe_layout())?;
    if canonical != candidate
        || canonical.parent() != Some(root)
        || canonical.file_name() != Some(OsStr::new(name))
    {
        return Err(unsafe_layout());
    }
    Ok(canonical)
}

fn ensure_owned_directory(parent: &Path, name: &str) -> JournalResult<PathBuf> {
    let candidate = parent.join(name);
    match fs::symlink_metadata(&candidate) {
        Ok(metadata) if metadata.is_dir() && !metadata_is_reparse(&metadata) => {}
        Ok(_) => return Err(unsafe_layout()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir(&candidate).map_err(|_| io_error())?;
            sync_directory(parent)?;
        }
        Err(_) => return Err(io_error()),
    }
    let canonical = fs::canonicalize(&candidate).map_err(|_| unsafe_layout())?;
    if canonical != candidate || canonical.parent() != Some(parent) {
        return Err(unsafe_layout());
    }
    Ok(canonical)
}

fn require_missing(path: &Path) -> JournalResult<()> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(io_error()),
        Ok(_) => Err(invalid_intent()),
    }
}

fn read_bounded_regular_file(parent: &Path, path: &Path, limit: u64) -> JournalResult<Vec<u8>> {
    let metadata = fs::symlink_metadata(path).map_err(|_| io_error())?;
    if !metadata.is_file() || metadata_is_reparse(&metadata) || metadata.len() > limit {
        return Err(invalid_intent());
    }
    let canonical = fs::canonicalize(path).map_err(|_| io_error())?;
    if canonical != path || canonical.parent() != Some(parent) {
        return Err(invalid_intent());
    }
    let mut file = fs::File::open(path).map_err(|_| io_error())?;
    let opened = file.metadata().map_err(|_| io_error())?;
    if !same_file_metadata(&metadata, &opened) {
        return Err(invalid_intent());
    }
    let capacity = usize::try_from(metadata.len()).map_err(|_| limit_exceeded())?;
    let mut bytes = Vec::with_capacity(capacity);
    std::io::Read::by_ref(&mut file)
        .take(limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|_| io_error())?;
    if bytes.len() as u64 > limit
        || !same_file_metadata(&opened, &file.metadata().map_err(|_| io_error())?)
    {
        return Err(invalid_intent());
    }
    Ok(bytes)
}

fn remove_regular_file_if_present(parent: &Path, path: &Path) -> JournalResult<()> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => return Err(io_error()),
    };
    if !metadata.is_file() || metadata_is_reparse(&metadata) {
        return Err(invalid_intent());
    }
    let canonical = fs::canonicalize(path).map_err(|_| io_error())?;
    if canonical != path || canonical.parent() != Some(parent) {
        return Err(invalid_intent());
    }
    fs::remove_file(path).map_err(|_| io_error())
}

fn same_file_metadata(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    left.is_file() == right.is_file()
        && left.len() == right.len()
        && left.modified().ok() == right.modified().ok()
        && file_identity(left) == file_identity(right)
}

#[cfg(unix)]
fn file_identity(metadata: &fs::Metadata) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;

    Some((metadata.dev(), metadata.ino()))
}

#[cfg(windows)]
fn file_identity(metadata: &fs::Metadata) -> Option<(u64, u64)> {
    let _ = metadata;
    None
}

#[cfg(not(any(unix, windows)))]
fn file_identity(_metadata: &fs::Metadata) -> Option<(u64, u64)> {
    None
}

#[cfg(windows)]
fn sync_directory(path: &Path) -> JournalResult<()> {
    use std::os::windows::fs::OpenOptionsExt;

    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    let directory = fs::OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)
        .map_err(|_| io_error())?;
    // Windows does not support FlushFileBuffers for directory handles. The
    // handle validation above is paired with MOVEFILE_WRITE_THROUGH for every
    // journal rename; file payloads themselves are flushed before the move.
    let _ = directory.sync_all();
    Ok(())
}

#[cfg(not(windows))]
fn sync_directory(path: &Path) -> JournalResult<()> {
    fs::File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|_| io_error())
}

#[cfg(windows)]
pub(crate) fn metadata_is_reparse(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
pub(crate) fn metadata_is_reparse(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

pub(crate) fn sync_owned_directory(path: &Path) -> JournalResult<()> {
    sync_directory(path)
}

#[cfg(windows)]
pub(crate) fn rename_owned_atomic(source: &Path, destination: &Path) -> JournalResult<()> {
    use std::os::windows::ffi::OsStrExt;

    const MOVEFILE_WRITE_THROUGH: u32 = 0x0000_0008;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        #[link_name = "MoveFileExW"]
        fn move_file_ex_w(existing: *const u16, target: *const u16, flags: u32) -> i32;
    }

    let mut source = source.as_os_str().encode_wide().collect::<Vec<_>>();
    source.push(0);
    let mut destination = destination.as_os_str().encode_wide().collect::<Vec<_>>();
    destination.push(0);
    // SAFETY: both path buffers are live, NUL-terminated UTF-16 strings. No
    // replace flag is used, so an existing destination is never overwritten.
    let moved = unsafe {
        move_file_ex_w(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_WRITE_THROUGH,
        )
    };
    if moved == 0 { Err(io_error()) } else { Ok(()) }
}

#[cfg(not(windows))]
pub(crate) fn rename_owned_atomic(source: &Path, destination: &Path) -> JournalResult<()> {
    fs::rename(source, destination).map_err(|_| io_error())
}

const fn unsafe_layout() -> JournalError {
    JournalError::new(JournalErrorCode::UnsafeLayout)
}

const fn invalid_intent() -> JournalError {
    JournalError::new(JournalErrorCode::InvalidIntent)
}

const fn limit_exceeded() -> JournalError {
    JournalError::new(JournalErrorCode::LimitExceeded)
}

const fn io_error() -> JournalError {
    JournalError::new(JournalErrorCode::Io)
}

#[cfg(test)]
#[path = "journal_test.rs"]
mod tests;
