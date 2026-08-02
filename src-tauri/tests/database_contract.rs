mod common;

use sqlx::Row;
use textbooklens_lib::db::Database;
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
fn recovery_marks_interrupted_imports_as_failed() {
    let temp_dir = tempfile::tempdir().unwrap();
    let database = Database::open(temp_dir.path().join("library.sqlite3")).unwrap();
    let book_id = Uuid::new_v4().to_string();

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
    });

    textbooklens_lib::db::settings::recover_interrupted_imports(database.pool()).unwrap();

    tauri::async_runtime::block_on(async {
        let status = sqlx::query(
            "SELECT import_status, import_error_code, import_error_message FROM books WHERE id = ?",
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
            "上次导入被应用退出中断，请重试"
        );
    });
}
