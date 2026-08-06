use std::fs;

use tempfile::TempDir;

use super::*;
use crate::{
    app_state::AppPaths,
    maintenance::storage::{APP_DATA_DIRECTORY_NAME, prepare_app_data_paths},
};

#[test]
fn strict_intent_round_trips_and_commit_cleanup_is_durable() {
    let fixture = JournalFixture::new();
    let journal_id = Uuid::new_v4();
    let book_id = Uuid::new_v4();
    let page_id = Uuid::new_v4();
    let journal = DeleteJournal::from_sources(
        journal_id,
        book_id,
        vec![
            RelativePathToken::new(format!("books/{book_id}")).unwrap(),
            RelativePathToken::new(format!("cache/indexing-pages/{page_id}")).unwrap(),
        ],
    )
    .unwrap();

    fixture
        .store
        .write_intent(&journal, &NoJournalFault)
        .unwrap();
    fixture
        .store
        .write_commit_marker(journal_id, &NoJournalFault)
        .unwrap();

    assert_eq!(fixture.store.list_intents().unwrap(), vec![journal.clone()]);
    assert_eq!(fixture.store.read_intent(journal_id).unwrap(), journal);
    fixture
        .store
        .remove_journal_files(journal_id, &NoJournalFault)
        .unwrap();
    assert!(fixture.store.list_intents().unwrap().is_empty());
}

#[test]
fn tokens_reject_escape_alias_device_and_noncanonical_forms() {
    let book_id = Uuid::new_v4();
    for token in [
        format!("/books/{book_id}"),
        format!("books/{book_id}/../outside"),
        format!(r"books\{book_id}"),
        format!("C:/books/{book_id}"),
        format!("//server/share/books/{book_id}"),
        format!("books/{}/NUL", book_id.to_string().to_ascii_uppercase()),
        "cache/indexing-pages/con".to_owned(),
        "cache//indexing-pages/page".to_owned(),
    ] {
        assert_eq!(
            RelativePathToken::new(token).unwrap_err().code,
            JournalErrorCode::InvalidIntent
        );
    }
}

#[test]
fn strict_reader_rejects_unknown_fields_duplicate_sources_and_oversize_files() {
    let fixture = JournalFixture::new();
    let journal_id = Uuid::new_v4();
    let book_id = Uuid::new_v4();
    let intent_path = fixture.store.intents.join(format!("{journal_id}.json"));
    fs::write(
        &intent_path,
        format!(
            r#"{{"format":"{DELETE_INTENT_FORMAT}","version":1,"journalId":"{journal_id}","bookId":"{book_id}","entries":[],"title":"private sentinel"}}"#
        ),
    )
    .unwrap();
    assert_eq!(
        fixture.store.list_intents().unwrap_err().code,
        JournalErrorCode::InvalidIntent
    );

    fs::remove_file(&intent_path).unwrap();
    let duplicate = RelativePathToken::new(format!("books/{book_id}")).unwrap();
    assert_eq!(
        DeleteJournal::from_sources(journal_id, book_id, vec![duplicate.clone(), duplicate])
            .unwrap_err()
            .code,
        JournalErrorCode::InvalidIntent
    );

    fs::write(
        &intent_path,
        vec![b'x'; MAX_DELETE_JOURNAL_BYTES as usize + 1],
    )
    .unwrap();
    assert_eq!(
        fixture.store.read_intent(journal_id).unwrap_err().code,
        JournalErrorCode::InvalidIntent
    );
}

#[test]
fn orphan_or_malformed_commit_markers_fail_closed() {
    let fixture = JournalFixture::new();
    let journal_id = Uuid::new_v4();
    fs::write(
        fixture
            .store
            .intents
            .join(format!("{journal_id}.committed")),
        serde_json::to_vec(&DeleteCommitMarker::new(journal_id)).unwrap(),
    )
    .unwrap();
    assert_eq!(
        fixture.store.list_intents().unwrap_err().code,
        JournalErrorCode::InvalidIntent
    );

    fs::remove_file(
        fixture
            .store
            .intents
            .join(format!("{journal_id}.committed")),
    )
    .unwrap();
    fs::write(fixture.store.intents.join("not-a-uuid.committed"), b"{}").unwrap();
    assert_eq!(
        fixture.store.list_intents().unwrap_err().code,
        JournalErrorCode::InvalidIntent
    );
}

#[test]
fn every_atomic_write_boundary_leaves_only_a_valid_intent_or_removable_temp() {
    for boundary in JournalBoundary::ALL.into_iter().filter(|boundary| {
        matches!(
            boundary,
            JournalBoundary::IntentTempCreated
                | JournalBoundary::IntentTempWritten
                | JournalBoundary::IntentTempSynced
                | JournalBoundary::IntentRenamed
                | JournalBoundary::IntentParentSynced
        )
    }) {
        let fixture = JournalFixture::new();
        let journal_id = Uuid::new_v4();
        let book_id = Uuid::new_v4();
        let journal = DeleteJournal::from_sources(
            journal_id,
            book_id,
            vec![RelativePathToken::new(format!("books/{book_id}")).unwrap()],
        )
        .unwrap();
        let error = fixture
            .store
            .write_intent(&journal, &FailAt(boundary))
            .unwrap_err();
        assert!(matches!(error, JournalOperationError::Injected(_)));

        fixture.store.cleanup_stale_temporary_files().unwrap();
        let intent = fixture.store.intents.join(format!("{journal_id}.json"));
        if intent.exists() {
            assert_eq!(fixture.store.read_intent(journal_id).unwrap(), journal);
            fixture
                .store
                .remove_journal_files(journal_id, &NoJournalFault)
                .unwrap();
        }
        assert!(fixture.store.list_intents().unwrap().is_empty());
    }
}

#[test]
fn debug_output_redacts_identifiers_and_paths() {
    let fixture = JournalFixture::new();
    let journal_id = Uuid::new_v4();
    let book_id = Uuid::new_v4();
    let journal = DeleteJournal::from_sources(
        journal_id,
        book_id,
        vec![RelativePathToken::new(format!("books/{book_id}")).unwrap()],
    )
    .unwrap();
    let rendered = format!("{journal:?} {:?}", fixture.store);
    assert!(!rendered.contains(&journal_id.to_string()));
    assert!(!rendered.contains(&book_id.to_string()));
    assert!(!rendered.contains(fixture.paths.root.to_string_lossy().as_ref()));
}

#[cfg(windows)]
#[test]
fn intent_junction_is_rejected_without_reading_its_target() {
    use std::process::Command;

    let fixture = JournalFixture::new();
    let journal_id = Uuid::new_v4();
    let outside = fixture.temporary.path().join("outside-intent");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("private-sentinel"), b"untouched").unwrap();
    let link = fixture.store.intents.join(format!("{journal_id}.json"));
    let output = Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(&link)
        .arg(&outside)
        .output()
        .unwrap();
    assert!(output.status.success());

    assert_eq!(
        fixture.store.read_intent(journal_id).unwrap_err().code,
        JournalErrorCode::InvalidIntent
    );
    assert_eq!(
        fs::read(outside.join("private-sentinel")).unwrap(),
        b"untouched"
    );
}

struct FailAt(JournalBoundary);

impl JournalFaultInjector for FailAt {
    fn checkpoint(&self, boundary: JournalBoundary) -> Result<(), JournalInjectedCrash> {
        if boundary == self.0 {
            Err(JournalInjectedCrash::at(boundary))
        } else {
            Ok(())
        }
    }
}

struct JournalFixture {
    temporary: TempDir,
    paths: AppPaths,
    store: JournalStore,
}

impl JournalFixture {
    fn new() -> Self {
        let temporary = TempDir::new().unwrap();
        let prepared =
            prepare_app_data_paths(&temporary.path().join(APP_DATA_DIRECTORY_NAME)).unwrap();
        let paths = AppPaths {
            root: prepared.root,
            books: prepared.books,
            cache: prepared.cache,
            logs: prepared.logs,
            database: prepared.database,
        };
        let store = JournalStore::open(&paths).unwrap();
        Self {
            temporary,
            paths,
            store,
        }
    }
}
