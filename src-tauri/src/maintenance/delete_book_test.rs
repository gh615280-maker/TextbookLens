use std::fs;

use tempfile::TempDir;

use super::*;
use crate::maintenance::storage::{APP_DATA_DIRECTORY_NAME, prepare_app_data_paths};

#[test]
fn book_and_scratch_shape_validation_is_strict() {
    assert!(validate_target_shape(TargetKind::BookExact("pdf"), "", EntryKind::Directory).is_ok());
    assert!(
        validate_target_shape(
            TargetKind::BookExact("pdf"),
            "original.pdf",
            EntryKind::File
        )
        .is_ok()
    );
    assert!(
        validate_target_shape(
            TargetKind::BookExact("pdf"),
            "derived/document.html",
            EntryKind::File
        )
        .is_ok()
    );
    for (relative, kind) in [
        ("alternate.pdf", EntryKind::File),
        ("original.epub", EntryKind::File),
        ("derived", EntryKind::File),
        ("../outside", EntryKind::Directory),
        ("derived/NUL.txt", EntryKind::File),
        ("derived/alias?.bin", EntryKind::File),
    ] {
        assert!(validate_target_shape(TargetKind::BookExact("pdf"), relative, kind).is_err());
    }

    let attempt_id = Uuid::new_v4();
    assert!(
        validate_target_shape(
            TargetKind::PageScratch,
            &format!("{attempt_id}.render"),
            EntryKind::File
        )
        .is_ok()
    );
    assert!(
        validate_target_shape(
            TargetKind::PageScratch,
            &format!("nested/{attempt_id}.render"),
            EntryKind::File
        )
        .is_err()
    );
}

#[test]
fn fault_matrix_covers_journal_moves_database_commit_and_finalize() {
    let matrix = DeleteBoundary::fault_matrix(2);
    for boundary in JournalBoundary::ALL {
        assert!(matrix.contains(&DeleteBoundary::Journal(boundary)));
    }
    for step in CompleteDeleteStep::ALL {
        assert!(matrix.contains(&DeleteBoundary::Database(step)));
    }
    for index in 0..2 {
        assert!(matrix.contains(&DeleteBoundary::BeforeStageMove(index)));
        assert!(matrix.contains(&DeleteBoundary::StageMoved(index)));
        assert!(matrix.contains(&DeleteBoundary::StageSourceParentSynced(index)));
        assert!(matrix.contains(&DeleteBoundary::StageTrashParentSynced(index)));
        assert!(matrix.contains(&DeleteBoundary::StageVerified(index)));
        assert!(matrix.contains(&DeleteBoundary::AfterStageMove(index)));
        assert!(matrix.contains(&DeleteBoundary::BeforeFinalizeEntry(index)));
        assert!(matrix.contains(&DeleteBoundary::FinalizeEntryRemoved(index)));
        assert!(matrix.contains(&DeleteBoundary::FinalizeEntryParentSynced(index)));
        assert!(matrix.contains(&DeleteBoundary::AfterFinalizeEntry(index)));
    }
    assert!(matrix.contains(&DeleteBoundary::BeforeTrashRootCreate));
    assert!(matrix.contains(&DeleteBoundary::BeforeTrashRootRemove));
    assert!(!DeleteBoundary::Database(CompleteDeleteStep::BeforeCommit).is_after_database_commit());
    assert!(!DeleteBoundary::StageTrashParentSynced(0).is_after_database_commit());
    assert!(DeleteBoundary::Database(CompleteDeleteStep::AfterCommit).is_after_database_commit());
    assert!(DeleteBoundary::FinalizeEntryRemoved(0).is_after_database_commit());
}

#[test]
fn duplicate_file_identity_is_rejected_before_journaling() {
    let fixture = DeletePathFixture::new();
    let book_id = Uuid::new_v4();
    let book = fixture.paths.books.join(book_id.to_string());
    let derived = book.join("derived");
    fs::create_dir_all(&derived).unwrap();
    let original = book.join("original.pdf");
    fs::write(&original, b"same app-owned bytes").unwrap();
    fs::hard_link(&original, derived.join("alias.bin")).unwrap();
    let plan = plan(book_id);

    let error = collect_delete_targets(&fixture.paths, &fixture.store, &plan).unwrap_err();

    assert_eq!(error.code, AppErrorCode::InvalidInput);
    assert!(original.exists());
}

#[test]
fn service_and_error_debug_output_never_include_paths_or_book_ids() {
    let fixture = DeletePathFixture::new();
    let book_id = Uuid::new_v4();
    let service = DeleteBookService::new(
        fixture.database.pool().clone(),
        fixture.paths.clone(),
        MaintenanceGate::default(),
    );
    let error = DeleteBookError::Injected(DeleteBoundary::BeforeStageMove(0));
    let rendered = format!("{service:?} {error:?}");
    assert!(!rendered.contains(&book_id.to_string()));
    assert!(!rendered.contains(fixture.paths.root.to_string_lossy().as_ref()));
}

fn plan(book_id: Uuid) -> BookDeletePlan {
    BookDeletePlan {
        book_id,
        format: BookFormat::Pdf,
        import_status: ImportStatus::Ready,
        stored_path: Some(format!("books/{book_id}/original.pdf")),
        page_ids: Vec::new(),
        has_nonterminal_indexing: false,
    }
}

struct DeletePathFixture {
    _temporary: TempDir,
    paths: AppPaths,
    store: JournalStore,
    database: crate::db::Database,
}

impl DeletePathFixture {
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
        let database = crate::db::Database::open(&paths.database).unwrap();
        let store = JournalStore::open(&paths).unwrap();
        Self {
            _temporary: temporary,
            paths,
            store,
            database,
        }
    }
}
