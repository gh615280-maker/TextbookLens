mod common;

use sqlx::{
    Row,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use textbooklens_lib::{app_state::AppPaths, db::Database};
use uuid::Uuid;

#[test]
fn migration_creates_the_local_database_contract() {
    let temp_dir = tempfile::tempdir().unwrap();
    let database = Database::open(temp_dir.path().join("library.sqlite3")).unwrap();
    let pool = database.pool();

    tauri::async_runtime::block_on(async {
        let tables: Vec<String> = sqlx::query_scalar(
            "SELECT name FROM sqlite_master WHERE type IN ('table', 'view') ORDER BY name",
        )
        .fetch_all(pool)
        .await
        .unwrap();
        for table in [
            "_sqlx_migrations",
            "annotations",
            "app_settings",
            "blocks",
            "books",
            "conversations",
            "messages",
            "provider_profiles",
            "search_chunks",
            "search_chunks_fts",
            "sections",
        ] {
            assert!(
                tables.iter().any(|actual| actual == table),
                "missing {table}"
            );
        }

        let foreign_keys: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
            .fetch_one(pool)
            .await
            .unwrap();
        assert_eq!(foreign_keys, 1);

        let settings = sqlx::query(
            "SELECT onboarding_completed, theme, context_mode, ui_language FROM app_settings WHERE id = 1",
        )
        .fetch_one(pool)
        .await
        .unwrap();
        assert_eq!(settings.get::<i64, _>("onboarding_completed"), 0);
        assert_eq!(settings.get::<String, _>("theme"), "system");
        assert_eq!(settings.get::<String, _>("context_mode"), "standard");
        assert_eq!(settings.get::<String, _>("ui_language"), "zh-CN");

        let book_id = Uuid::new_v4().to_string();
        let section_id = Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO books (id, sha256, title, format, original_filename, stored_path, import_status, created_at, updated_at) VALUES (?, ?, ?, 'pdf', ?, ?, 'ready', ?, ?)",
        )
        .bind(&book_id)
        .bind("a".repeat(64))
        .bind("Contract book")
        .bind("contract.pdf")
        .bind("books/contract.pdf")
        .bind(common::utc_timestamp())
        .bind(common::utc_timestamp())
        .execute(pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO sections (id, book_id, ordinal, title, locator_json) VALUES (?, ?, 0, ?, '{}')",
        )
        .bind(&section_id)
        .bind(&book_id)
        .bind("Chapter 1")
        .execute(pool)
        .await
        .unwrap();

        for (ordinal, kind) in [
            "heading",
            "paragraph",
            "list",
            "table",
            "caption",
            "equation",
        ]
        .into_iter()
        .enumerate()
        {
            sqlx::query(
                "INSERT INTO blocks (id, book_id, section_id, ordinal, kind, plain_text, locator_json) VALUES (?, ?, ?, ?, ?, 'text', '{}')",
            )
            .bind(Uuid::new_v4().to_string())
            .bind(&book_id)
            .bind(&section_id)
            .bind(ordinal as i64)
            .bind(kind)
            .execute(pool)
            .await
            .unwrap();
        }
        let invalid_block_kind = sqlx::query(
            "INSERT INTO blocks (id, book_id, section_id, ordinal, kind, plain_text, locator_json) VALUES (?, ?, ?, 99, 'code', 'unsafe legacy kind', '{}')",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(&book_id)
        .bind(&section_id)
        .execute(pool)
        .await;
        assert!(
            invalid_block_kind.is_err(),
            "blocks.kind must accept only the canonical six kinds"
        );

        let invalid_note = sqlx::query(
            "INSERT INTO annotations (id, book_id, section_id, kind, anchor_json, selected_text, created_at, updated_at) VALUES (?, ?, ?, 'note', '{}', 'selected', ?, ?)",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(&book_id)
        .bind(&section_id)
        .bind(common::utc_timestamp())
        .bind(common::utc_timestamp())
        .execute(pool)
        .await;
        assert!(
            invalid_note.is_err(),
            "note without note_text must violate CHECK"
        );

        let conversation_id = Uuid::new_v4().to_string();
        let annotation_id = Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO conversations (id, book_id, section_id, scope, anchor_json, selected_text, created_at, updated_at) VALUES (?, ?, ?, 'selection', '{}', 'selected', ?, ?)",
        )
        .bind(&conversation_id)
        .bind(&book_id)
        .bind(&section_id)
        .bind(common::utc_timestamp())
        .bind(common::utc_timestamp())
        .execute(pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO annotations (id, book_id, section_id, kind, conversation_id, created_at, updated_at) VALUES (?, ?, ?, 'ai_conversation', ?, ?, ?)",
        )
        .bind(&annotation_id)
        .bind(&book_id)
        .bind(&section_id)
        .bind(&conversation_id)
        .bind(common::utc_timestamp())
        .bind(common::utc_timestamp())
        .execute(pool)
        .await
        .unwrap();
        sqlx::query("DELETE FROM annotations WHERE id = ?")
            .bind(&annotation_id)
            .execute(pool)
            .await
            .unwrap();
        let conversation_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM conversations WHERE id = ?")
                .bind(&conversation_id)
                .fetch_one(pool)
                .await
                .unwrap();
        assert_eq!(conversation_count, 0);

        sqlx::query(
            "INSERT INTO search_chunks(id, book_id, section_id, ordinal, text, locator_json, token_estimate) VALUES(?, ?, ?, 0, '线性代数 matrix', '{}', 10)",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(&book_id)
        .bind(&section_id)
        .execute(pool)
        .await
        .unwrap();
        let fts_text: String = sqlx::query_scalar(
            "SELECT text FROM search_chunks_fts WHERE search_chunks_fts MATCH '线性代'",
        )
        .fetch_one(pool)
        .await
        .unwrap();
        assert_eq!(fts_text, "线性代数 matrix");

        sqlx::query("DELETE FROM books WHERE id = ?")
            .bind(&book_id)
            .execute(pool)
            .await
            .unwrap();
        let sections_remaining: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sections")
            .fetch_one(pool)
            .await
            .unwrap();
        assert_eq!(sections_remaining, 0);
    });
}

#[test]
fn phase_two_books_contract_supports_queued_null_claims_and_error_stage() {
    let temp_dir = tempfile::tempdir().unwrap();
    let database = Database::open(temp_dir.path().join("library.sqlite3")).unwrap();
    let book_id = Uuid::new_v4().to_string();

    tauri::async_runtime::block_on(async {
        sqlx::query(
            "INSERT INTO books (id, title, format, original_filename, import_status, created_at, updated_at) VALUES (?, ?, 'pdf', ?, 'queued', ?, ?)",
        )
        .bind(&book_id)
        .bind("Queued book")
        .bind("queued.pdf")
        .bind(common::utc_timestamp())
        .bind(common::utc_timestamp())
        .execute(database.pool())
        .await
        .unwrap();

        sqlx::query(
            "UPDATE books SET import_status = 'failed', import_error_code = 'LOCAL_IO_ERROR', import_error_message = 'copy failed', import_error_stage = 'copying' WHERE id = ?",
        )
        .bind(&book_id)
        .execute(database.pool())
        .await
        .unwrap();

        let row = sqlx::query(
            "SELECT sha256, stored_path, import_status, import_error_stage FROM books WHERE id = ?",
        )
        .bind(&book_id)
        .fetch_one(database.pool())
        .await
        .unwrap();
        assert_eq!(row.get::<Option<String>, _>("sha256"), None);
        assert_eq!(row.get::<Option<String>, _>("stored_path"), None);
        assert_eq!(row.get::<String, _>("import_status"), "failed");
        assert_eq!(
            row.get::<Option<String>, _>("import_error_stage")
                .as_deref(),
            Some("copying")
        );
    });
}

#[test]
fn phase_two_migration_upgrades_phase_one_rows_without_placeholders() {
    let temp_dir = tempfile::tempdir().unwrap();
    let database_path = temp_dir.path().join("upgrade.sqlite3");
    let book_id = Uuid::new_v4().to_string();
    let ready_id = Uuid::new_v4().to_string();
    let section_id = Uuid::new_v4().to_string();

    tauri::async_runtime::block_on(async {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                SqliteConnectOptions::new()
                    .filename(&database_path)
                    .create_if_missing(true)
                    .foreign_keys(true),
            )
            .await
            .unwrap();
        sqlx::raw_sql(include_str!("../migrations/0001_initial.sql"))
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO books (id, sha256, title, format, original_filename, stored_path, import_status, created_at, updated_at) VALUES (?, ?, 'Interrupted', 'pdf', 'old.pdf', '', 'copying', ?, ?)",
        )
        .bind(&book_id)
        .bind(format!("pending:{book_id}"))
        .bind(common::utc_timestamp())
        .bind(common::utc_timestamp())
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO books (id, sha256, title, format, original_filename, stored_path, import_status, created_at, updated_at) VALUES (?, ?, 'Ready', 'pdf', 'ready.pdf', ?, 'ready', ?, ?)",
        )
        .bind(&ready_id)
        .bind("d".repeat(64))
        .bind(format!("books/{ready_id}/original.pdf"))
        .bind(common::utc_timestamp())
        .bind(common::utc_timestamp())
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO sections (id, book_id, ordinal, title, locator_json) VALUES (?, ?, 0, 'Chapter', '{}')",
        )
        .bind(&section_id)
        .bind(&ready_id)
        .execute(&pool)
        .await
        .unwrap();

        sqlx::raw_sql(include_str!("../migrations/0002_import_lifecycle.sql"))
            .execute(&pool)
            .await
            .unwrap();

        let upgraded = sqlx::query("SELECT sha256, stored_path FROM books WHERE id = ?")
            .bind(&book_id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(upgraded.get::<Option<String>, _>("sha256"), None);
        assert_eq!(upgraded.get::<Option<String>, _>("stored_path"), None);
        let preserved_sections: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM sections WHERE book_id = ?")
                .bind(&ready_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(preserved_sections, 1);

        sqlx::query(
            "INSERT INTO books (id, title, format, original_filename, import_status, created_at, updated_at) VALUES (?, 'Queued', 'epub', 'queued.epub', 'queued', ?, ?)",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(common::utc_timestamp())
        .bind(common::utc_timestamp())
        .execute(&pool)
        .await
        .unwrap();
        let foreign_key_violations: Vec<(String, i64, String, i64)> =
            sqlx::query_as("PRAGMA foreign_key_check")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert!(foreign_key_violations.is_empty());
        sqlx::query("DELETE FROM books WHERE id = ?")
            .bind(&ready_id)
            .execute(&pool)
            .await
            .unwrap();
        let cascaded_sections: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM sections WHERE id = ?")
                .bind(&section_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(cascaded_sections, 0);
    });
}

#[test]
fn recovery_marks_interrupted_imports_as_failed() {
    let temp_dir = tempfile::tempdir().unwrap();
    let root = temp_dir.path().join("app-data");
    let paths = AppPaths {
        books: root.join("books"),
        cache: root.join("cache"),
        logs: root.join("logs"),
        database: root.join("library.sqlite3"),
        root,
    };
    for directory in [&paths.root, &paths.books, &paths.cache, &paths.logs] {
        std::fs::create_dir_all(directory).unwrap();
    }
    let database = Database::open(&paths.database).unwrap();
    let book_id = Uuid::new_v4().to_string();
    let parsing_id = Uuid::new_v4().to_string();
    let orphan_id = Uuid::new_v4();

    tauri::async_runtime::block_on(async {
        sqlx::query(
            "INSERT INTO books (id, sha256, title, format, original_filename, stored_path, import_status, created_at, updated_at) VALUES (?, ?, ?, 'pdf', ?, ?, 'copying', ?, ?)",
        )
        .bind(&book_id)
        .bind("b".repeat(64))
        .bind("Interrupted book")
        .bind("interrupted.pdf")
        .bind("books/interrupted.pdf")
        .bind(common::utc_timestamp())
        .bind(common::utc_timestamp())
        .execute(database.pool())
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO books (id, sha256, title, format, original_filename, stored_path, import_status, created_at, updated_at) VALUES (?, ?, ?, 'pdf', ?, ?, 'parsing', ?, ?)",
        )
        .bind(&parsing_id)
        .bind("c".repeat(64))
        .bind("Parsing book")
        .bind("parsing.pdf")
        .bind(format!("books/{parsing_id}/original.pdf"))
        .bind(common::utc_timestamp())
        .bind(common::utc_timestamp())
        .execute(database.pool())
        .await
        .unwrap();
    });

    let copying_directory = paths.books.join(&book_id);
    std::fs::create_dir_all(&copying_directory).unwrap();
    std::fs::write(copying_directory.join("original.pdf.partial"), b"partial").unwrap();
    let parsing_directory = paths.books.join(&parsing_id);
    std::fs::create_dir_all(parsing_directory.join("derived")).unwrap();
    std::fs::write(parsing_directory.join("original.pdf"), b"owned").unwrap();
    std::fs::write(
        parsing_directory.join("derived/document.html.partial"),
        b"partial",
    )
    .unwrap();
    let orphan_directory = paths.books.join(orphan_id.to_string());
    std::fs::create_dir_all(&orphan_directory).unwrap();
    std::fs::write(orphan_directory.join("original.pdf"), b"orphan").unwrap();

    textbooklens_lib::db::settings::recover_interrupted_imports(database.pool(), &paths).unwrap();

    tauri::async_runtime::block_on(async {
        let status = sqlx::query(
            "SELECT import_status, import_error_code, import_error_message, import_error_stage, sha256, stored_path FROM books WHERE id = ?",
        )
        .bind(&book_id)
        .fetch_one(database.pool())
        .await
        .unwrap();
        assert_eq!(status.get::<String, _>("import_status"), "failed");
        assert_eq!(
            status.get::<String, _>("import_error_code"),
            "IMPORT_CANCELLED"
        );
        assert_eq!(
            status.get::<String, _>("import_error_message"),
            "上次导入因应用退出而中断，请重试"
        );
        assert_eq!(status.get::<String, _>("import_error_stage"), "copying");
        assert_eq!(status.get::<Option<String>, _>("sha256"), None);
        assert_eq!(status.get::<Option<String>, _>("stored_path"), None);

        let parsing = sqlx::query(
            "SELECT import_status, import_error_stage, sha256, stored_path FROM books WHERE id = ?",
        )
        .bind(&parsing_id)
        .fetch_one(database.pool())
        .await
        .unwrap();
        assert_eq!(parsing.get::<String, _>("import_status"), "failed");
        assert_eq!(parsing.get::<String, _>("import_error_stage"), "parsing");
        assert_eq!(parsing.get::<String, _>("sha256"), "c".repeat(64));
        assert_eq!(
            parsing.get::<String, _>("stored_path"),
            format!("books/{parsing_id}/original.pdf")
        );
    });
    assert!(!copying_directory.exists());
    assert!(!orphan_directory.exists());
    assert!(parsing_directory.join("original.pdf").exists());
    assert!(
        !parsing_directory
            .join("derived/document.html.partial")
            .exists()
    );
}
