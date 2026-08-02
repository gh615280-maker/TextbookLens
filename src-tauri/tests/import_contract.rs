use std::{fs, sync::Arc};

use sha2::{Digest, Sha256};
use sqlx::Row;
use textbooklens_lib::{
    app_state::AppPaths,
    db::Database,
    documents::import::{
        BeginImportOutcome, BeginImportRequest, ImportEvent, ImportService, ImportStage,
    },
    errors::AppErrorCode,
};
use tokio_util::sync::CancellationToken;

async fn test_service() -> (tempfile::TempDir, Database, ImportService) {
    tokio::task::spawn_blocking(|| {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("app-data");
        let paths = AppPaths {
            books: root.join("books"),
            cache: root.join("cache"),
            logs: root.join("logs"),
            database: root.join("library.sqlite3"),
            root,
        };
        for path in [&paths.root, &paths.books, &paths.cache, &paths.logs] {
            fs::create_dir_all(path).unwrap();
        }
        let database = Database::open(&paths.database).unwrap();
        let service = ImportService::new(database.pool().clone(), paths);
        (temp, database, service)
    })
    .await
    .unwrap()
}

fn source_file(temp: &tempfile::TempDir, name: &str, bytes: &[u8]) -> std::path::PathBuf {
    let path = temp.path().join(name);
    fs::write(&path, bytes).unwrap();
    path
}

fn created_book(outcome: BeginImportOutcome) -> textbooklens_lib::domain::BookSummary {
    match outcome {
        BeginImportOutcome::Created { book } => book,
        BeginImportOutcome::Duplicate { .. } => panic!("expected a newly owned book"),
    }
}

fn no_progress(_: ImportEvent) {}

#[tokio::test]
async fn copied_book_survives_source_deletion() {
    let (temp, _database, service) = test_service().await;
    let expected = b"%PDF-1.7\nowned textbook bytes\n%%EOF";
    let source = source_file(&temp, "source.pdf", expected);

    let book = created_book(
        service
            .begin_import(
                BeginImportRequest::new(source.to_string_lossy().into_owned()),
                Arc::new(no_progress),
            )
            .await
            .unwrap(),
    );
    fs::remove_file(&source).unwrap();

    let internal = service.read_book_source(book.id).await.unwrap();
    assert_eq!(internal, expected);
}

#[tokio::test]
async fn duplicate_hash_returns_existing_book_without_new_directory() {
    let (temp, database, service) = test_service().await;
    let bytes = b"PK\x03\x04same epub bytes";
    let first = source_file(&temp, "first.epub", bytes);
    let second = source_file(&temp, "renamed.epub", bytes);

    let existing = created_book(
        service
            .begin_import(
                BeginImportRequest::new(first.to_string_lossy().into_owned()),
                Arc::new(no_progress),
            )
            .await
            .unwrap(),
    );
    let duplicate = service
        .begin_import(
            BeginImportRequest::new(second.to_string_lossy().into_owned()),
            Arc::new(no_progress),
        )
        .await
        .unwrap();

    match duplicate {
        BeginImportOutcome::Duplicate { book } => assert_eq!(book.id, existing.id),
        BeginImportOutcome::Created { .. } => panic!("same bytes must deduplicate"),
    }
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM books")
        .fetch_one(database.pool())
        .await
        .unwrap();
    assert_eq!(count, 1);
    assert_eq!(
        fs::read_dir(service.paths().books.clone()).unwrap().count(),
        1
    );
}

#[tokio::test]
async fn cancelling_copy_removes_partial_internal_file_and_keeps_source() {
    let (temp, database, service) = test_service().await;
    let bytes = vec![0x5a; 3 * 1024 * 1024];
    let source = source_file(&temp, "large.docx", &bytes);
    let expected_hash = Sha256::digest(&bytes);
    let cancellation = CancellationToken::new();
    let cancellation_after_chunk = cancellation.clone();
    let progress = Arc::new(move |event: ImportEvent| {
        if event.completed >= 1024 * 1024 {
            cancellation_after_chunk.cancel();
        }
    });

    let error = service
        .begin_import_with_token(
            BeginImportRequest::new(source.to_string_lossy().into_owned()),
            cancellation,
            progress,
        )
        .await
        .unwrap_err();

    assert_eq!(error.code, AppErrorCode::ImportCancelled);
    assert!(source.exists());
    assert_eq!(Sha256::digest(fs::read(&source).unwrap()), expected_hash);
    let failed =
        sqlx::query("SELECT import_status, import_error_code, stored_path FROM books LIMIT 1")
            .fetch_one(database.pool())
            .await
            .unwrap();
    assert_eq!(failed.get::<String, _>("import_status"), "failed");
    assert_eq!(
        failed.get::<String, _>("import_error_code"),
        "IMPORT_CANCELLED"
    );
    assert_eq!(failed.get::<String, _>("stored_path"), "");
    assert_eq!(
        fs::read_dir(service.paths().books.clone()).unwrap().count(),
        0
    );
}

#[tokio::test]
async fn concurrent_same_bytes_converge_without_orphan_directory() {
    let (temp, database, service) = test_service().await;
    let bytes = vec![0x31; 2 * 1024 * 1024];
    let first = source_file(&temp, "one.pdf", &bytes);
    let second = source_file(&temp, "two.PDF", &bytes);
    let first_service = service.clone();
    let second_service = service.clone();

    let (left, right) = tokio::join!(
        first_service.begin_import(
            BeginImportRequest::new(first.to_string_lossy().into_owned()),
            Arc::new(no_progress),
        ),
        second_service.begin_import(
            BeginImportRequest::new(second.to_string_lossy().into_owned()),
            Arc::new(no_progress),
        )
    );
    let outcomes = [left.unwrap(), right.unwrap()];
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| matches!(outcome, BeginImportOutcome::Created { .. }))
            .count(),
        1
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| matches!(outcome, BeginImportOutcome::Duplicate { .. }))
            .count(),
        1
    );
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM books")
        .fetch_one(database.pool())
        .await
        .unwrap();
    assert_eq!(count, 1);
    assert_eq!(
        fs::read_dir(service.paths().books.clone()).unwrap().count(),
        1
    );
}

#[tokio::test]
async fn validates_extensions_and_never_persists_or_reports_source_paths() {
    let (temp, database, service) = test_service().await;
    let upper = source_file(&temp, "UPPER.DOCX", b"PK\x03\x04docx");
    let book = created_book(
        service
            .begin_import(
                BeginImportRequest::new(upper.to_string_lossy().into_owned()),
                Arc::new(no_progress),
            )
            .await
            .unwrap(),
    );
    let filename: String = sqlx::query_scalar("SELECT original_filename FROM books WHERE id = ?")
        .bind(book.id.to_string())
        .fetch_one(database.pool())
        .await
        .unwrap();
    assert_eq!(filename, "UPPER.DOCX");
    assert!(!filename.contains(['/', '\\']));

    let unsupported = source_file(&temp, "private.txt", b"not supported");
    let error = service
        .begin_import(
            BeginImportRequest::new(unsupported.to_string_lossy().into_owned()),
            Arc::new(no_progress),
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, AppErrorCode::UnsupportedFileType);
    assert!(
        !error
            .to_string()
            .contains(unsupported.to_string_lossy().as_ref())
    );
}

#[tokio::test]
async fn cancellation_command_is_idempotent() {
    let (_temp, _database, service) = test_service().await;
    let unknown = uuid::Uuid::new_v4();
    service.cancel_import(unknown).await.unwrap();
    service.cancel_import(unknown).await.unwrap();

    let registry = service.cancellations();
    assert!(registry.lock().is_empty());
}

#[tokio::test]
async fn cancellation_racing_finalize_never_leaves_ready_or_internal_bytes() {
    let (temp, database, service) = test_service().await;
    let source = source_file(&temp, "race.pdf", b"%PDF cancellation race");
    let book = created_book(
        service
            .begin_import(
                BeginImportRequest::new(source.to_string_lossy().into_owned()),
                Arc::new(no_progress),
            )
            .await
            .unwrap(),
    );
    let registry = service.cancellations();
    let cancel_during_finalize = Arc::new(move |event: ImportEvent| {
        if event.stage == ImportStage::Indexing && event.completed == 0 {
            registry.lock().get(&book.id).unwrap().cancel();
        }
    });

    let error = service
        .finalize_import(book.id, cancel_during_finalize)
        .await
        .unwrap_err();
    assert_eq!(error.code, AppErrorCode::ImportCancelled);
    let status: String = sqlx::query_scalar("SELECT import_status FROM books WHERE id = ?")
        .bind(book.id.to_string())
        .fetch_one(database.pool())
        .await
        .unwrap();
    assert_eq!(status, "failed");
    assert!(!service.paths().books.join(book.id.to_string()).exists());
    assert!(source.exists());
}

#[tokio::test]
async fn parse_failure_retries_owned_copy_and_failed_delete_preserves_source() {
    let (temp, database, service) = test_service().await;
    let expected = b"PK\x03\x04retryable docx";
    let source = source_file(&temp, "retry.docx", expected);
    let book = created_book(
        service
            .begin_import(
                BeginImportRequest::new(source.to_string_lossy().into_owned()),
                Arc::new(no_progress),
            )
            .await
            .unwrap(),
    );
    service
        .mark_import_failed(book.id, ImportStage::Parsing, AppErrorCode::FileCorrupted)
        .await
        .unwrap();

    let retried = created_book(
        service
            .retry_import(book.id, None, Arc::new(no_progress))
            .await
            .unwrap(),
    );
    assert_eq!(retried.id, book.id);
    assert_eq!(service.read_book_source(book.id).await.unwrap(), expected);

    service.cancel_import(book.id).await.unwrap();
    service.delete_failed_import(book.id).await.unwrap();
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM books WHERE id = ?")
        .bind(book.id.to_string())
        .fetch_one(database.pool())
        .await
        .unwrap();
    assert_eq!(count, 0);
    assert_eq!(fs::read(&source).unwrap(), expected);
}
