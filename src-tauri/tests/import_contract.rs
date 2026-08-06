use std::{fs, sync::Arc};

use sha2::{Digest, Sha256};
use sqlx::Row;
use textbooklens_lib::{
    app_state::AppPaths,
    db::Database,
    documents::import::{
        BeginImportOutcome, BeginImportRequest, ImportCancellationRegistry, ImportEvent,
        ImportService, ImportStage, ParsedBookMetadata,
    },
    documents::storage::{copy_source, noop_progress, validate_source},
    domain::{ActiveOperationKind, BookIndexAggregateStatus, ImportErrorStage, ImportStatus},
    errors::AppErrorCode,
    maintenance::{
        delete_book::DeleteBookService,
        gate::MaintenanceGate,
        storage::{APP_DATA_DIRECTORY_NAME, prepare_app_data_paths},
    },
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
async fn complete_delete_waits_for_explicit_import_cancel_ack_and_preserves_user_source() {
    let (temp, paths, database) = tokio::task::spawn_blocking(|| {
        let temp = tempfile::tempdir().unwrap();
        let prepared = prepare_app_data_paths(&temp.path().join(APP_DATA_DIRECTORY_NAME)).unwrap();
        let paths = AppPaths {
            root: prepared.root,
            books: prepared.books,
            cache: prepared.cache,
            logs: prepared.logs,
            database: prepared.database,
        };
        let database = Database::open(&paths.database).unwrap();
        (temp, paths, database)
    })
    .await
    .unwrap();
    let gate = MaintenanceGate::default();
    let imports = ImportService::with_maintenance_gate(
        database.pool().clone(),
        paths.clone(),
        ImportCancellationRegistry::default(),
        gate.clone(),
    );
    let deletion = DeleteBookService::new(database.pool().clone(), paths, gate);
    let user_bytes = b"%PDF synthetic user-owned original";
    let user_source = source_file(&temp, "user-original.pdf", user_bytes);
    let book = created_book(
        imports
            .begin_import(
                BeginImportRequest::new(user_source.to_string_lossy().into_owned()),
                Arc::new(no_progress),
            )
            .await
            .unwrap(),
    );

    assert_eq!(
        deletion.delete_book(book.id).await.unwrap_err().code,
        AppErrorCode::RequestConflict
    );
    assert!(
        sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM books WHERE id = ?)")
            .bind(book.id.to_string())
            .fetch_one(database.pool())
            .await
            .unwrap()
    );

    imports.cancel_import(book.id).await.unwrap();
    assert_eq!(
        imports.get_book(book.id).await.unwrap().import_status,
        ImportStatus::Failed
    );
    deletion.delete_failed_book(book.id).await.unwrap();

    assert!(
        !sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM books WHERE id = ?)")
            .bind(book.id.to_string())
            .fetch_one(database.pool())
            .await
            .unwrap()
    );
    assert_eq!(fs::read(user_source).unwrap(), user_bytes);
}

#[tokio::test]
async fn queued_status_maps_through_the_rust_book_summary() {
    let (_temp, database, service) = test_service().await;
    let book_id = uuid::Uuid::new_v4();
    sqlx::query(
        "INSERT INTO books (id, title, format, original_filename, import_status, created_at, updated_at) VALUES (?, 'Queued', 'pdf', 'queued.pdf', 'queued', ?, ?)",
    )
    .bind(book_id.to_string())
    .bind("2026-08-02T00:00:00Z")
    .bind("2026-08-02T00:00:00Z")
    .execute(database.pool())
    .await
    .unwrap();

    assert_eq!(
        service.get_book(book_id).await.unwrap().import_status,
        ImportStatus::Queued
    );
}

#[tokio::test]
async fn book_summaries_project_the_latest_safe_index_aggregate_per_book() {
    let (_temp, database, service) = test_service().await;
    let first_book = uuid::Uuid::new_v4();
    let second_book = uuid::Uuid::new_v4();
    let older_run = uuid::Uuid::new_v4();
    let latest_run = uuid::Uuid::new_v4();
    let second_run = uuid::Uuid::new_v4();
    let timestamp = "2026-08-04T00:00:00.000Z";
    let later_timestamp = "2026-08-04T00:00:01.000Z";
    let first_hash = "a".repeat(64);
    let second_hash = "b".repeat(64);
    let content_hash = "c".repeat(64);

    for (book_id, title, filename, hash) in [
        (first_book, "First", "first.pdf", &first_hash),
        (second_book, "Second", "second.pdf", &second_hash),
    ] {
        sqlx::query(
            "INSERT INTO books (id, title, format, original_filename, sha256, stored_path, import_status, created_at, updated_at) VALUES (?, ?, 'pdf', ?, ?, 'owned/original.pdf', 'ready', ?, ?)",
        )
        .bind(book_id.to_string())
        .bind(title)
        .bind(filename)
        .bind(hash)
        .bind(timestamp)
        .bind(timestamp)
        .execute(database.pool())
        .await
        .unwrap();
    }
    for (run_id, book_id, hash, updated_at) in [
        (older_run, first_book, &first_hash, timestamp),
        (latest_run, first_book, &first_hash, later_timestamp),
        (second_run, second_book, &second_hash, later_timestamp),
    ] {
        sqlx::query(
            "INSERT INTO index_runs (id, book_id, source_sha256, provider_kind, model_id, analysis_schema_version, render_version, parser_version, status, created_at, updated_at) VALUES (?, ?, ?, 'openai', 'model', 'v1', 'v1', 'v1', 'running', ?, ?)",
        )
        .bind(run_id.to_string())
        .bind(book_id.to_string())
        .bind(hash)
        .bind(updated_at)
        .bind(updated_at)
        .execute(database.pool())
        .await
        .unwrap();
    }
    let attempt = uuid::Uuid::new_v4().to_string();
    for (
        run_id,
        book_id,
        page_number,
        status,
        review_reason,
        safe_error_code,
        safe_error_message,
        content_version,
    ) in [
        (
            latest_run, first_book, 1_i64, "indexed", None, None, None, 1_i64,
        ),
        (
            latest_run,
            first_book,
            2_i64,
            "needs_review",
            Some("incomplete_content"),
            None,
            None,
            1_i64,
        ),
        (
            latest_run,
            first_book,
            3_i64,
            "failed",
            None,
            Some("INDEX_PROVIDER_FAILED"),
            Some("Provider request failed safely."),
            0_i64,
        ),
        (
            second_run,
            second_book,
            1_i64,
            "failed",
            None,
            Some("INDEX_PROVIDER_FAILED"),
            Some("Provider request failed safely."),
            0_i64,
        ),
    ] {
        sqlx::query(
            "INSERT INTO index_pages (id, run_id, book_id, page_number, quality_reason, status, attempt_id, attempt_count, safe_error_code, safe_error_message, review_reason_code, content_sha256, content_version, created_at, updated_at) VALUES (?, ?, ?, ?, 'no_text', ?, ?, 1, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(uuid::Uuid::new_v4().to_string())
        .bind(run_id.to_string())
        .bind(book_id.to_string())
        .bind(page_number)
        .bind(status)
        .bind(&attempt)
        .bind(safe_error_code)
        .bind(safe_error_message)
        .bind(review_reason)
        .bind(if content_version > 0 { Some(&content_hash) } else { None })
        .bind(content_version)
        .bind(later_timestamp)
        .bind(later_timestamp)
        .execute(database.pool())
        .await
        .unwrap();
    }

    let summaries = service.list_books().await.unwrap();
    let first = summaries.iter().find(|book| book.id == first_book).unwrap();
    assert_eq!(
        first.index_aggregate.status,
        BookIndexAggregateStatus::Partial
    );
    assert_eq!(first.index_aggregate.total_pages, 3);
    assert_eq!(first.index_aggregate.indexed_pages, 1);
    assert_eq!(first.index_aggregate.review_pages, 1);
    assert_eq!(first.index_aggregate.failed_pages, 1);
    let second = summaries
        .iter()
        .find(|book| book.id == second_book)
        .unwrap();
    assert_eq!(
        second.index_aggregate.status,
        BookIndexAggregateStatus::Failed
    );
    assert_eq!(second.index_aggregate.total_pages, 1);
    assert_eq!(second.index_aggregate.failed_pages, 1);
}

#[tokio::test]
async fn books_without_page_indexes_are_safely_not_required_and_corruption_fails_closed() {
    let (_temp, database, service) = test_service().await;
    let book_id = uuid::Uuid::new_v4();
    sqlx::query(
        "INSERT INTO books (id, title, format, original_filename, import_status, created_at, updated_at) VALUES (?, 'Safe', 'pdf', 'safe.pdf', 'ready', '2026-08-04T00:00:00.000Z', '2026-08-04T00:00:00.000Z')",
    )
    .bind(book_id.to_string())
    .execute(database.pool())
    .await
    .unwrap();
    let summary = service.get_book(book_id).await.unwrap();
    assert_eq!(
        summary.index_aggregate.status,
        BookIndexAggregateStatus::NotRequired
    );
    assert_eq!(summary.index_aggregate.total_pages, 0);

    let hash = "d".repeat(64);
    let run_id = uuid::Uuid::new_v4();
    sqlx::query("UPDATE books SET sha256 = ?, stored_path = 'owned/safe.pdf' WHERE id = ?")
        .bind(&hash)
        .bind(book_id.to_string())
        .execute(database.pool())
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO index_runs (id, book_id, source_sha256, provider_kind, model_id, analysis_schema_version, render_version, parser_version, status, created_at, updated_at) VALUES (?, ?, ?, 'openai', 'model', 'v1', 'v1', 'v1', 'running', '2026-08-04T00:00:01.000Z', '2026-08-04T00:00:01.000Z')",
    )
    .bind(run_id.to_string())
    .bind(book_id.to_string())
    .bind(&hash)
    .execute(database.pool())
    .await
    .unwrap();
    let mut connection = database.pool().acquire().await.unwrap();
    sqlx::query("PRAGMA ignore_check_constraints = ON")
        .execute(&mut *connection)
        .await
        .unwrap();
    let corrupt_insert = sqlx::query(
        "INSERT INTO index_pages (id, run_id, book_id, page_number, quality_reason, status, attempt_id, attempt_count, created_at, updated_at) VALUES (?, ?, ?, 1, 'no_text', 'corrupt', ?, 1, '2026-08-04T00:00:01.000Z', '2026-08-04T00:00:01.000Z')",
    )
    .bind(uuid::Uuid::new_v4().to_string())
    .bind(run_id.to_string())
    .bind(book_id.to_string())
    .bind(uuid::Uuid::new_v4().to_string())
    .execute(&mut *connection)
    .await;
    let restore = sqlx::query("PRAGMA ignore_check_constraints = OFF")
        .execute(&mut *connection)
        .await;
    let restored = match restore {
        Ok(_) => {
            sqlx::query_scalar::<_, i64>("PRAGMA ignore_check_constraints")
                .fetch_one(&mut *connection)
                .await
        }
        Err(error) => Err(error),
    };
    if !matches!(restored, Ok(0)) {
        let _ = connection.close().await;
        panic!("failed to restore SQLite CHECK constraints: {restored:?}");
    }
    corrupt_insert.unwrap();
    drop(connection);

    assert_eq!(
        service.get_book(book_id).await.unwrap_err().code,
        AppErrorCode::DatabaseError
    );
}

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
async fn owned_copy_bytes_and_hash_allow_parsing_after_source_deletion() {
    let (temp, database, service) = test_service().await;
    let expected = b"%PDF-1.7\nsynthetic owned copy contract\n%%EOF";
    let source = source_file(&temp, "synthetic.pdf", expected);
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

    let owned = service.read_book_source(book.id).await.unwrap();
    assert_eq!(owned, expected);
    assert_eq!(owned.len(), expected.len());
    let stored_hash: String = sqlx::query_scalar("SELECT sha256 FROM books WHERE id = ?")
        .bind(book.id.to_string())
        .fetch_one(database.pool())
        .await
        .unwrap();
    assert_eq!(
        stored_hash,
        Sha256::digest(expected)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    );

    service
        .begin_parse(
            book.id,
            ParsedBookMetadata {
                title: "Synthetic PDF".to_owned(),
                author: None,
                language: None,
            },
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn source_reads_fail_closed_when_the_owned_copy_hash_changes() {
    let (temp, _database, service) = test_service().await;
    let source = source_file(&temp, "synthetic.pdf", b"%PDF-1.7\nsource\n%%EOF");
    let book = created_book(
        service
            .begin_import(
                BeginImportRequest::new(source.to_string_lossy().into_owned()),
                Arc::new(no_progress),
            )
            .await
            .unwrap(),
    );
    let owned_path = service
        .paths()
        .books
        .join(book.id.to_string())
        .join("original.pdf");
    fs::write(owned_path, b"%PDF-1.7\nchanged\n%%EOF").unwrap();

    assert_eq!(
        service.read_book_source(book.id).await.unwrap_err().code,
        AppErrorCode::FileCorrupted
    );
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
    let failed = sqlx::query(
        "SELECT import_status, import_error_code, import_error_stage, sha256, stored_path FROM books LIMIT 1",
    )
            .fetch_one(database.pool())
            .await
            .unwrap();
    assert_eq!(failed.get::<String, _>("import_status"), "failed");
    assert_eq!(
        failed.get::<String, _>("import_error_code"),
        "IMPORT_CANCELLED"
    );
    assert_eq!(
        failed
            .get::<Option<String>, _>("import_error_stage")
            .as_deref(),
        Some("copying")
    );
    assert_eq!(failed.get::<Option<String>, _>("sha256"), None);
    assert_eq!(failed.get::<Option<String>, _>("stored_path"), None);
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
    assert!(registry.is_empty());
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
            registry.cancel(book.id);
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancel_command_racing_finalize_returns_cancelled_and_compensates_once() {
    let (temp, database, service) = test_service().await;
    let source = source_file(&temp, "command-race.pdf", b"%PDF command cancellation race");
    let book = created_book(
        service
            .begin_import(
                BeginImportRequest::new(source.to_string_lossy().into_owned()),
                Arc::new(no_progress),
            )
            .await
            .unwrap(),
    );
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (cancelled_tx, cancelled_rx) = std::sync::mpsc::channel();
    let cancelled_rx = Arc::new(std::sync::Mutex::new(cancelled_rx));
    let progress = Arc::new(move |event: ImportEvent| {
        if event.stage == ImportStage::Indexing && event.completed == 0 {
            started_tx.send(()).unwrap();
            cancelled_rx.lock().unwrap().recv().unwrap();
        }
    });
    let cancel_service = service.clone();
    let cancel = tokio::spawn(async move {
        started_rx.recv().unwrap();
        let result = cancel_service.cancel_import(book.id).await;
        cancelled_tx.send(()).unwrap();
        result
    });

    let error = service
        .finalize_import(book.id, progress)
        .await
        .unwrap_err();
    cancel.await.unwrap().unwrap();
    assert_eq!(error.code, AppErrorCode::ImportCancelled);
    let row = sqlx::query(
        "SELECT import_status, import_error_code, import_error_stage, sha256, stored_path FROM books WHERE id = ?",
    )
    .bind(book.id.to_string())
    .fetch_one(database.pool())
    .await
    .unwrap();
    assert_eq!(row.get::<String, _>("import_status"), "failed");
    assert_eq!(
        row.get::<String, _>("import_error_code"),
        "IMPORT_CANCELLED"
    );
    assert_eq!(row.get::<String, _>("import_error_stage"), "indexing");
    assert_eq!(row.get::<Option<String>, _>("sha256"), None);
    assert_eq!(row.get::<Option<String>, _>("stored_path"), None);
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
    let book_directory = service.paths().books.join(book.id.to_string());
    let derived_directory = book_directory.join("derived");
    fs::write(derived_directory.join("document.html"), b"derived").unwrap();
    fs::write(derived_directory.join(".document.html.partial"), b"partial").unwrap();
    service
        .mark_import_failed(book.id, ImportStage::Parsing, AppErrorCode::FileCorrupted)
        .await
        .unwrap();
    assert!(book_directory.join("original.docx").exists());
    assert!(!derived_directory.join("document.html").exists());
    assert!(!derived_directory.join(".document.html.partial").exists());
    let failed_summary = service.get_book(book.id).await.unwrap();
    assert_eq!(
        failed_summary.import_error_stage,
        Some(ImportErrorStage::Parsing)
    );
    assert_eq!(
        failed_summary.import_error_message.as_deref(),
        Some(AppErrorCode::FileCorrupted.user_message())
    );

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

#[cfg(windows)]
#[tokio::test]
async fn parse_failure_removes_a_derived_junction_without_touching_its_destination() {
    let (temp, _database, service) = test_service().await;
    let source = source_file(&temp, "junction.docx", b"PK\x03\x04junction docx");
    let book = created_book(
        service
            .begin_import(
                BeginImportRequest::new(source.to_string_lossy().into_owned()),
                Arc::new(no_progress),
            )
            .await
            .unwrap(),
    );
    let book_directory = service.paths().books.join(book.id.to_string());
    let derived = book_directory.join("derived");
    fs::remove_dir(&derived).unwrap();
    let outside = temp.path().join("outside-derived");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("keep.html"), b"keep").unwrap();
    let output = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(&derived)
        .arg(&outside)
        .output()
        .unwrap();
    assert!(output.status.success());

    service
        .mark_import_failed(book.id, ImportStage::Parsing, AppErrorCode::FileCorrupted)
        .await
        .unwrap();

    assert_eq!(fs::read(outside.join("keep.html")).unwrap(), b"keep");
    assert!(derived.is_dir());
    assert!(fs::read_dir(&derived).unwrap().next().is_none());
    assert!(book_directory.join("original.docx").exists());
}

#[tokio::test]
async fn copied_hash_is_the_actual_sha256_without_pending_workarounds() {
    let (temp, database, service) = test_service().await;
    let bytes = b"%PDF-1.7\nactual hash contract\n%%EOF";
    let source = source_file(&temp, "actual.pdf", bytes);
    let book = created_book(
        service
            .begin_import(
                BeginImportRequest::new(source.to_string_lossy().into_owned()),
                Arc::new(no_progress),
            )
            .await
            .unwrap(),
    );

    let row = sqlx::query("SELECT sha256, stored_path FROM books WHERE id = ?")
        .bind(book.id.to_string())
        .fetch_one(database.pool())
        .await
        .unwrap();
    assert_eq!(
        row.get::<String, _>("sha256"),
        Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    );
    assert_eq!(
        row.get::<String, _>("stored_path"),
        format!("books/{}/original.pdf", book.id)
    );
}

#[tokio::test]
async fn non_sha_constraint_failure_is_not_reported_as_duplicate() {
    let (temp, database, service) = test_service().await;
    let bytes = b"%PDF duplicate candidate";
    let first = source_file(&temp, "first.pdf", bytes);
    let second = source_file(&temp, "second.pdf", bytes);
    created_book(
        service
            .begin_import(
                BeginImportRequest::new(first.to_string_lossy().into_owned()),
                Arc::new(no_progress),
            )
            .await
            .unwrap(),
    );
    sqlx::query(
        "CREATE TRIGGER reject_second_claim BEFORE UPDATE OF sha256 ON books WHEN NEW.original_filename = 'second.pdf' BEGIN SELECT RAISE(ABORT, 'claim blocked'); END",
    )
    .execute(database.pool())
    .await
    .unwrap();

    let error = service
        .begin_import(
            BeginImportRequest::new(second.to_string_lossy().into_owned()),
            Arc::new(no_progress),
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, AppErrorCode::DatabaseError);
}

#[tokio::test]
async fn source_reads_reject_escape_and_wrong_internal_copy_paths() {
    let (temp, database, service) = test_service().await;
    let source = source_file(&temp, "owned.pdf", b"%PDF owned source");
    let book = created_book(
        service
            .begin_import(
                BeginImportRequest::new(source.to_string_lossy().into_owned()),
                Arc::new(no_progress),
            )
            .await
            .unwrap(),
    );
    let book_directory = service.paths().books.join(book.id.to_string());
    fs::write(book_directory.join("alternate.pdf"), b"wrong internal copy").unwrap();

    for untrusted_path in [
        format!("books/{}/../escape.pdf", book.id),
        format!("books/{}/alternate.pdf", book.id),
    ] {
        sqlx::query("UPDATE books SET stored_path = ? WHERE id = ?")
            .bind(untrusted_path)
            .bind(book.id.to_string())
            .execute(database.pool())
            .await
            .unwrap();
        let error = service.read_book_source(book.id).await.unwrap_err();
        assert_eq!(error.code, AppErrorCode::InvalidInput);
    }
}

#[tokio::test]
async fn source_partial_is_create_new_and_never_overwrites_residue() {
    let (temp, _database, service) = test_service().await;
    let source_path = source_file(&temp, "residue.pdf", b"%PDF new bytes");
    let source = validate_source(source_path.to_str().unwrap()).unwrap();
    let book_id = uuid::Uuid::new_v4();
    let book_directory = service.paths().books.join(book_id.to_string());
    fs::create_dir_all(book_directory.join("derived")).unwrap();
    let partial = book_directory.join("original.pdf.partial");
    fs::write(&partial, b"do not overwrite").unwrap();

    let error = copy_source(
        service.paths(),
        book_id,
        &source,
        CancellationToken::new(),
        noop_progress(),
    )
    .await
    .unwrap_err();
    assert_eq!(error.code, AppErrorCode::LocalIoError);
    assert_eq!(fs::read(partial).unwrap(), b"do not overwrite");
}

#[test]
fn retry_old_cleanup_cannot_remove_or_cancel_the_new_attempt_token() {
    let registry = ImportCancellationRegistry::default();
    let maintenance_gate = MaintenanceGate::default();
    let book_id = uuid::Uuid::new_v4();
    let old_token = CancellationToken::new();
    let new_token = CancellationToken::new();
    let old_attempt = registry.register_with_permit(
        book_id,
        old_token.clone(),
        maintenance_gate
            .try_acquire_normal(ActiveOperationKind::Import)
            .unwrap(),
    );
    let cleanup_registry = registry.clone();
    let cleanup_barrier = Arc::new(std::sync::Barrier::new(2));
    let cleanup_started = cleanup_barrier.clone();
    let cleanup = std::thread::spawn(move || {
        cleanup_started.wait();
        cleanup_registry.remove_if_owner(book_id, old_attempt)
    });
    let _new_attempt = registry.register_with_permit(
        book_id,
        new_token.clone(),
        maintenance_gate
            .try_acquire_normal(ActiveOperationKind::Import)
            .unwrap(),
    );

    cleanup_barrier.wait();
    assert!(!cleanup.join().unwrap());
    assert!(!new_token.is_cancelled());
    registry.cancel(book_id);
    assert!(new_token.is_cancelled());
    assert!(!old_token.is_cancelled());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn retry_started_before_old_finalize_cleanup_keeps_new_copy_and_token() {
    let (temp, database, service) = test_service().await;
    let original = source_file(&temp, "old.pdf", b"%PDF old attempt");
    let replacement_bytes = b"%PDF replacement attempt";
    let replacement = source_file(&temp, "replacement.pdf", replacement_bytes);
    let book = created_book(
        service
            .begin_import(
                BeginImportRequest::new(original.to_string_lossy().into_owned()),
                Arc::new(no_progress),
            )
            .await
            .unwrap(),
    );
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (resume_tx, resume_rx) = std::sync::mpsc::channel();
    let resume_rx = Arc::new(std::sync::Mutex::new(resume_rx));
    let progress = Arc::new(move |event: ImportEvent| {
        if event.stage == ImportStage::Indexing && event.completed == 0 {
            started_tx.send(()).unwrap();
            resume_rx.lock().unwrap().recv().unwrap();
        }
    });
    let finalize_service = service.clone();
    let old_finalize =
        tokio::spawn(async move { finalize_service.finalize_import(book.id, progress).await });

    started_rx.recv().unwrap();
    service.cancel_import(book.id).await.unwrap();
    let retried = created_book(
        service
            .retry_import(
                book.id,
                Some(replacement.to_string_lossy().into_owned()),
                Arc::new(no_progress),
            )
            .await
            .unwrap(),
    );
    assert_eq!(retried.id, book.id);
    resume_tx.send(()).unwrap();
    let old_error = old_finalize.await.unwrap().unwrap_err();
    assert_eq!(old_error.code, AppErrorCode::ImportCancelled);

    assert_eq!(
        service.read_book_source(book.id).await.unwrap(),
        replacement_bytes
    );
    assert!(!service.cancellations().is_empty());
    let status: String = sqlx::query_scalar("SELECT import_status FROM books WHERE id = ?")
        .bind(book.id.to_string())
        .fetch_one(database.pool())
        .await
        .unwrap();
    assert_eq!(status, "parsing");
    assert!(original.exists());
    assert!(replacement.exists());
}

#[tokio::test]
async fn retry_rejects_a_db_selected_internal_file_even_when_its_hash_matches() {
    let (temp, database, service) = test_service().await;
    let expected = b"%PDF retry ownership";
    let source = source_file(&temp, "retry-owned.pdf", expected);
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
    let book_directory = service.paths().books.join(book.id.to_string());
    fs::write(book_directory.join("alternate.pdf"), expected).unwrap();
    sqlx::query("UPDATE books SET stored_path = ? WHERE id = ?")
        .bind(format!("books/{}/alternate.pdf", book.id))
        .bind(book.id.to_string())
        .execute(database.pool())
        .await
        .unwrap();

    let error = service
        .retry_import(book.id, None, Arc::new(no_progress))
        .await
        .unwrap_err();
    assert_eq!(error.code, AppErrorCode::InvalidInput);
}

#[tokio::test]
async fn replacement_retry_duplicate_is_explicit_and_never_persists_source_path() {
    let (temp, database, service) = test_service().await;
    let duplicate_bytes = b"PK\x03\x04existing epub";
    let first = source_file(&temp, "winner.epub", duplicate_bytes);
    let failed_source = source_file(&temp, "failed.epub", b"PK\x03\x04different epub");
    let replacement = source_file(&temp, "replacement.epub", duplicate_bytes);
    let winner = created_book(
        service
            .begin_import(
                BeginImportRequest::new(first.to_string_lossy().into_owned()),
                Arc::new(no_progress),
            )
            .await
            .unwrap(),
    );
    let failed = created_book(
        service
            .begin_import(
                BeginImportRequest::new(failed_source.to_string_lossy().into_owned()),
                Arc::new(no_progress),
            )
            .await
            .unwrap(),
    );
    service
        .mark_import_failed(failed.id, ImportStage::Parsing, AppErrorCode::FileCorrupted)
        .await
        .unwrap();

    let outcome = service
        .retry_import(
            failed.id,
            Some(replacement.to_string_lossy().into_owned()),
            Arc::new(no_progress),
        )
        .await
        .unwrap();
    match outcome {
        BeginImportOutcome::Duplicate { book } => assert_eq!(book.id, winner.id),
        BeginImportOutcome::Created { .. } => panic!("replacement duplicate must be explicit"),
    }
    let persisted_paths: Vec<Option<String>> = sqlx::query_scalar("SELECT stored_path FROM books")
        .fetch_all(database.pool())
        .await
        .unwrap();
    assert!(persisted_paths.iter().flatten().all(|path| {
        !path.contains(replacement.to_string_lossy().as_ref())
            && !path.contains(failed_source.to_string_lossy().as_ref())
    }));
}
