mod common;

use sqlx::{
    Row,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use textbooklens_lib::{
    app_state::AppPaths,
    db::{
        Database,
        indexing::{
            CreateIndexPage, CreateIndexRun, EncryptedRemoteResourceReference, create_index_page,
            create_index_run, find_current_run_aggregate_for_book, get_run_aggregate,
            track_remote_resource,
        },
    },
    domain::{
        ContentSource, IndexAggregateStatus, IndexPageStatus, IndexQualityReason,
        stable_index_page_block_id, stable_index_search_chunk_id,
    },
};
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
            "index_corrections",
            "index_page_blocks",
            "index_pages",
            "index_runs",
            "index_search_chunks",
            "index_search_chunks_fts",
            "messages",
            "panel_preferences",
            "provider_profiles",
            "provider_operation_consents",
            "provider_remote_resources",
            "search_chunks",
            "search_chunks_fts",
            "sections",
            "teaching_preferences",
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

        let teaching = sqlx::query(
            "SELECT instruction, revision, updated_at FROM teaching_preferences WHERE id = 1",
        )
        .fetch_one(pool)
        .await
        .unwrap();
        assert_eq!(teaching.get::<String, _>("instruction"), "");
        assert_eq!(teaching.get::<i64, _>("revision"), 0);
        let teaching_updated_at = teaching.get::<String, _>("updated_at");
        assert!(
            chrono::DateTime::parse_from_rfc3339(&teaching_updated_at).is_ok()
                && teaching_updated_at.ends_with('Z')
        );
        let teaching_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM teaching_preferences")
            .fetch_one(pool)
            .await
            .unwrap();
        assert_eq!(teaching_count, 1);

        let panel_preferences = sqlx::query(
            "SELECT last_x_ratio, last_y_ratio, width_px, height_px, updated_at FROM panel_preferences WHERE id = 1",
        )
        .fetch_one(pool)
        .await
        .unwrap();
        assert_eq!(panel_preferences.get::<f64, _>("last_x_ratio"), 0.5);
        assert_eq!(panel_preferences.get::<f64, _>("last_y_ratio"), 0.5);
        assert_eq!(panel_preferences.get::<f64, _>("width_px"), 400.0);
        assert_eq!(panel_preferences.get::<f64, _>("height_px"), 520.0);
        let panel_updated_at = panel_preferences.get::<String, _>("updated_at");
        assert_eq!(panel_updated_at.len(), 24);
        assert!(
            chrono::DateTime::parse_from_rfc3339(&panel_updated_at).is_ok()
                && panel_updated_at.ends_with('Z')
        );

        for invalid_statement in [
            "INSERT INTO teaching_preferences (id, updated_at) VALUES (2, '2026-08-04T00:00:00.000Z')",
            "UPDATE teaching_preferences SET revision = -1 WHERE id = 1",
            "UPDATE teaching_preferences SET updated_at = '2026-08-04 00:00:00' WHERE id = 1",
            "UPDATE teaching_preferences SET instruction = char(13) WHERE id = 1",
            "DELETE FROM teaching_preferences WHERE id = 1",
        ] {
            assert!(
                sqlx::query(invalid_statement).execute(pool).await.is_err(),
                "malformed teaching singleton state must be rejected: {invalid_statement}"
            );
        }

        let settings = sqlx::query(
            "SELECT onboarding_completed, default_learning_profile_id, default_vision_profile_id, theme, context_mode, ui_language, ui_language_initialized, first_reader_hint_completed FROM app_settings WHERE id = 1",
        )
        .fetch_one(pool)
        .await
        .unwrap();
        assert_eq!(settings.get::<i64, _>("onboarding_completed"), 0);
        assert_eq!(settings.get::<String, _>("theme"), "system");
        assert_eq!(settings.get::<String, _>("context_mode"), "standard");
        assert_eq!(settings.get::<String, _>("ui_language"), "zh-CN");
        assert_eq!(settings.get::<i64, _>("ui_language_initialized"), 1);
        assert_eq!(settings.get::<i64, _>("first_reader_hint_completed"), 0);
        assert_eq!(
            settings.get::<Option<String>, _>("default_learning_profile_id"),
            None
        );
        assert_eq!(
            settings.get::<Option<String>, _>("default_vision_profile_id"),
            None
        );

        let provider_columns: Vec<String> = sqlx::query_scalar(
            "SELECT name FROM pragma_table_info('provider_profiles') ORDER BY cid",
        )
        .fetch_all(pool)
        .await
        .unwrap();
        assert!(
            !provider_columns
                .iter()
                .any(|column| column == "credential_key")
        );

        let invalid_language =
            sqlx::query("UPDATE app_settings SET ui_language = 'fr' WHERE id = 1")
                .execute(pool)
                .await;
        assert!(
            invalid_language.is_err(),
            "ui_language must be a strict enum"
        );

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
            "INSERT INTO conversations (id, book_id, section_id, scope, anchor_kind, anchor_json, selected_text, created_at, updated_at) VALUES (?, ?, ?, 'selection', 'text', '{}', 'selected', ?, ?)",
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
fn learning_panels_forward_migration_preserves_history_and_reopens() {
    let temp_dir = tempfile::tempdir().unwrap();
    let database_path = temp_dir.path().join("learning-panels-forward.sqlite3");
    let timestamp = "2026-08-05T01:02:03.004Z";
    let book_id = Uuid::new_v4();
    let section_id = Uuid::new_v4();
    let selection_conversation_id = Uuid::new_v4();
    let book_conversation_id = Uuid::new_v4();
    let annotation_id = Uuid::new_v4();
    let user_message_id = Uuid::new_v4();
    let assistant_message_id = Uuid::new_v4();
    let region_conversation_id = Uuid::new_v4();

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
        apply_real_migrations_through_v10(&pool).await;

        sqlx::query(
            "INSERT INTO books (id, sha256, title, format, original_filename, stored_path, import_status, created_at, updated_at) VALUES (?, ?, 'Legacy history book', 'pdf', 'legacy.pdf', 'books/legacy/original.pdf', 'ready', ?, ?)",
        )
        .bind(book_id.to_string())
        .bind("b".repeat(64))
        .bind(timestamp)
        .bind(timestamp)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO sections (id, book_id, ordinal, title, locator_json) VALUES (?, ?, 0, 'Legacy section', '{}')",
        )
        .bind(section_id.to_string())
        .bind(book_id.to_string())
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO conversations (id, book_id, section_id, scope, anchor_json, selected_text, created_at, updated_at) VALUES (?, ?, ?, 'selection', '{\"legacy\":true}', 'legacy selected text', ?, ?)",
        )
        .bind(selection_conversation_id.to_string())
        .bind(book_id.to_string())
        .bind(section_id.to_string())
        .bind(timestamp)
        .bind(timestamp)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO conversations (id, book_id, scope, created_at, updated_at) VALUES (?, ?, 'book', ?, ?)",
        )
        .bind(book_conversation_id.to_string())
        .bind(book_id.to_string())
        .bind(timestamp)
        .bind(timestamp)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO messages (id, conversation_id, ordinal, role, action, content, created_at) VALUES (?, ?, 0, 'user', 'explain', 'legacy question', ?)",
        )
        .bind(user_message_id.to_string())
        .bind(selection_conversation_id.to_string())
        .bind(timestamp)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO messages (id, conversation_id, ordinal, role, action, content, provider_id, model_id, citations_json, created_at) VALUES (?, ?, 1, 'assistant', 'explain', 'legacy answer', 'openai', 'legacy-model', '[]', ?)",
        )
        .bind(assistant_message_id.to_string())
        .bind(selection_conversation_id.to_string())
        .bind(timestamp)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO annotations (id, book_id, section_id, kind, anchor_json, selected_text, conversation_id, created_at, updated_at) VALUES (?, ?, ?, 'ai_conversation', '{\"legacy\":true}', 'legacy selected text', ?, ?, ?)",
        )
        .bind(annotation_id.to_string())
        .bind(book_id.to_string())
        .bind(section_id.to_string())
        .bind(selection_conversation_id.to_string())
        .bind(timestamp)
        .bind(timestamp)
        .execute(&pool)
        .await
        .unwrap();

        sqlx::raw_sql(include_str!("../migrations/0011_learning_panels.sql"))
            .execute(&pool)
            .await
            .unwrap();

        let migrated = sqlx::query(
            "SELECT anchor_kind, anchor_json, selected_text, created_at, updated_at FROM conversations WHERE id = ?",
        )
        .bind(selection_conversation_id.to_string())
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(migrated.get::<String, _>("anchor_kind"), "text");
        assert_eq!(
            migrated.get::<String, _>("anchor_json"),
            "{\"legacy\":true}"
        );
        assert_eq!(
            migrated.get::<String, _>("selected_text"),
            "legacy selected text"
        );
        assert_eq!(migrated.get::<String, _>("created_at"), timestamp);
        assert_eq!(migrated.get::<String, _>("updated_at"), timestamp);
        assert_eq!(
            sqlx::query_scalar::<_, Option<String>>(
                "SELECT anchor_kind FROM conversations WHERE id = ?",
            )
            .bind(book_conversation_id.to_string())
            .fetch_one(&pool)
            .await
            .unwrap(),
            None
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM messages WHERE conversation_id = ?")
                .bind(selection_conversation_id.to_string())
                .fetch_one(&pool)
                .await
                .unwrap(),
            2
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT revision FROM annotations WHERE id = ?")
                .bind(annotation_id.to_string())
                .fetch_one(&pool)
                .await
                .unwrap(),
            1
        );

        sqlx::query(
            "INSERT INTO conversations (id, book_id, section_id, scope, anchor_kind, anchor_json, selected_text, created_at, updated_at) VALUES (?, ?, ?, 'selection', 'region', '{\"kind\":\"region\"}', NULL, ?, ?)",
        )
        .bind(region_conversation_id.to_string())
        .bind(book_id.to_string())
        .bind(section_id.to_string())
        .bind(timestamp)
        .bind(timestamp)
        .execute(&pool)
        .await
        .unwrap();

        for statement in [
            "INSERT INTO conversations (id, book_id, section_id, scope, anchor_kind, anchor_json, selected_text, created_at, updated_at) VALUES ('bad-text', ?, ?, 'selection', 'text', '{}', NULL, ?, ?)",
            "INSERT INTO conversations (id, book_id, section_id, scope, anchor_kind, anchor_json, selected_text, created_at, updated_at) VALUES ('bad-region-anchor', ?, ?, 'selection', 'region', NULL, NULL, ?, ?)",
            "INSERT INTO conversations (id, book_id, section_id, scope, anchor_kind, anchor_json, selected_text, created_at, updated_at) VALUES ('bad-region-text', ?, ?, 'selection', 'region', '{}', '   ', ?, ?)",
            "INSERT INTO conversations (id, book_id, section_id, scope, anchor_kind, anchor_json, selected_text, created_at, updated_at) VALUES ('bad-kind', ?, ?, 'selection', 'screen', '{}', 'text', ?, ?)",
        ] {
            assert!(
                sqlx::query(statement)
                    .bind(book_id.to_string())
                    .bind(section_id.to_string())
                    .bind(timestamp)
                    .bind(timestamp)
                    .execute(&pool)
                    .await
                    .is_err(),
                "inconsistent selection conversation must be rejected: {statement}"
            );
        }
        assert!(
            sqlx::query(
                "INSERT INTO conversations (id, book_id, scope, anchor_kind, created_at, updated_at) VALUES ('bad-book', ?, 'book', 'text', ?, ?)",
            )
            .bind(book_id.to_string())
            .bind(timestamp)
            .bind(timestamp)
            .execute(&pool)
            .await
            .is_err()
        );

        let panel = sqlx::query(
            "SELECT last_x_ratio, last_y_ratio, width_px, height_px, updated_at FROM panel_preferences WHERE id = 1",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(panel.get::<f64, _>("last_x_ratio"), 0.5);
        assert_eq!(panel.get::<f64, _>("last_y_ratio"), 0.5);
        assert_eq!(panel.get::<f64, _>("width_px"), 400.0);
        assert_eq!(panel.get::<f64, _>("height_px"), 520.0);
        assert_eq!(panel.get::<String, _>("updated_at").len(), 24);
        let panel_columns: Vec<String> = sqlx::query_scalar(
            "SELECT name FROM pragma_table_info('panel_preferences') ORDER BY cid",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(
            panel_columns,
            [
                "id",
                "last_x_ratio",
                "last_y_ratio",
                "width_px",
                "height_px",
                "updated_at"
            ]
        );
        for (statement, value) in [
            (
                "UPDATE panel_preferences SET last_x_ratio = ? WHERE id = 1",
                -0.1_f64,
            ),
            (
                "UPDATE panel_preferences SET last_x_ratio = ? WHERE id = 1",
                f64::NAN,
            ),
            (
                "UPDATE panel_preferences SET last_y_ratio = ? WHERE id = 1",
                f64::INFINITY,
            ),
            (
                "UPDATE panel_preferences SET width_px = ? WHERE id = 1",
                319.0,
            ),
            (
                "UPDATE panel_preferences SET height_px = ? WHERE id = 1",
                8193.0,
            ),
        ] {
            assert!(
                sqlx::query(statement)
                    .bind(value)
                    .execute(&pool)
                    .await
                    .is_err()
            );
        }
        assert!(
            sqlx::query(
                "UPDATE panel_preferences SET updated_at = '2026-08-05 01:02:03' WHERE id = 1",
            )
            .execute(&pool)
            .await
            .is_err()
        );
        assert!(
            sqlx::query("DELETE FROM panel_preferences WHERE id = 1")
                .execute(&pool)
                .await
                .is_err()
        );

        let foreign_key_violations: Vec<(String, i64, String, i64)> =
            sqlx::query_as("PRAGMA foreign_key_check")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert!(foreign_key_violations.is_empty());

        sqlx::query("DELETE FROM annotations WHERE id = ?")
            .bind(annotation_id.to_string())
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM conversations WHERE id = ?")
                .bind(selection_conversation_id.to_string())
                .fetch_one(&pool)
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM messages WHERE conversation_id = ?")
                .bind(selection_conversation_id.to_string())
                .fetch_one(&pool)
                .await
                .unwrap(),
            0
        );
        pool.close().await;
    });

    tauri::async_runtime::block_on(async {
        let reopened = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                SqliteConnectOptions::new()
                    .filename(&database_path)
                    .foreign_keys(true),
            )
            .await
            .unwrap();
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM panel_preferences")
                .fetch_one(&reopened)
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            sqlx::query_scalar::<_, String>("SELECT anchor_kind FROM conversations WHERE id = ?",)
                .bind(region_conversation_id.to_string())
                .fetch_one(&reopened)
                .await
                .unwrap(),
            "region"
        );
        let foreign_key_violations: Vec<(String, i64, String, i64)> =
            sqlx::query_as("PRAGMA foreign_key_check")
                .fetch_all(&reopened)
                .await
                .unwrap();
        assert!(foreign_key_violations.is_empty());
    });
}

#[test]
fn teaching_migration_is_forward_safe_preserves_rows_and_reopens_once() {
    let temp_dir = tempfile::tempdir().unwrap();
    let forward_path = temp_dir.path().join("forward.sqlite3");

    tauri::async_runtime::block_on(async {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                SqliteConnectOptions::new()
                    .filename(&forward_path)
                    .create_if_missing(true)
                    .foreign_keys(true),
            )
            .await
            .unwrap();
        for migration in [
            include_str!("../migrations/0001_initial.sql"),
            include_str!("../migrations/0002_import_lifecycle.sql"),
            include_str!("../migrations/0003_canonical_block_kinds.sql"),
            include_str!("../migrations/0004_reader_settings.sql"),
            include_str!("../migrations/0005_provider_profile_summary.sql"),
            include_str!("../migrations/0006_replanned_ui_preferences.sql"),
            include_str!("../migrations/0007_provider_operation_preferences.sql"),
        ] {
            sqlx::raw_sql(migration).execute(&pool).await.unwrap();
        }

        let book_id = Uuid::new_v4().to_string();
        let provider_id = Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO books (id, sha256, title, format, original_filename, stored_path, import_status, created_at, updated_at) VALUES (?, ?, 'Synthetic retained book', 'pdf', 'synthetic.pdf', ?, 'ready', ?, ?)",
        )
        .bind(&book_id)
        .bind("e".repeat(64))
        .bind(format!("books/{book_id}/original.pdf"))
        .bind(common::utc_timestamp())
        .bind(common::utc_timestamp())
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO provider_profiles (id, provider_kind, display_name, model_id, context_window_tokens, created_at, updated_at) VALUES (?, 'openai', 'Synthetic retained profile', 'synthetic-model', 32000, ?, ?)",
        )
        .bind(&provider_id)
        .bind(common::utc_timestamp())
        .bind(common::utc_timestamp())
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("UPDATE app_settings SET default_learning_profile_id = ? WHERE id = 1")
            .bind(&provider_id)
            .execute(&pool)
            .await
            .unwrap();

        sqlx::raw_sql(include_str!("../migrations/0008_teaching_instruction.sql"))
            .execute(&pool)
            .await
            .unwrap();

        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM books WHERE id = ?")
                .bind(&book_id)
                .fetch_one(&pool)
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            sqlx::query_scalar::<_, Option<String>>(
                "SELECT default_learning_profile_id FROM app_settings WHERE id = 1",
            )
            .fetch_one(&pool)
            .await
            .unwrap()
            .as_deref(),
            Some(provider_id.as_str())
        );
        let foreign_key_violations: Vec<(String, i64, String, i64)> =
            sqlx::query_as("PRAGMA foreign_key_check")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert!(foreign_key_violations.is_empty());
    });

    let reopen_path = temp_dir.path().join("reopen.sqlite3");
    {
        let database = Database::open(&reopen_path).unwrap();
        assert_eq!(
            tauri::async_runtime::block_on(
                sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM teaching_preferences",)
                    .fetch_one(database.pool())
            )
            .unwrap(),
            1
        );
    }
    let reopened = Database::open(&reopen_path).unwrap();
    assert_eq!(
        tauri::async_runtime::block_on(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM teaching_preferences",)
                .fetch_one(reopened.pool())
        )
        .unwrap(),
        1
    );
}

#[test]
fn ui_preference_migration_preserves_reader_settings_and_profile_foreign_keys() {
    let temp_dir = tempfile::tempdir().unwrap();
    let options = SqliteConnectOptions::new()
        .filename(temp_dir.path().join("legacy.sqlite3"))
        .create_if_missing(true)
        .foreign_keys(true);
    let pool = tauri::async_runtime::block_on(
        SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options),
    )
    .unwrap();

    tauri::async_runtime::block_on(async {
        for migration in [
            include_str!("../migrations/0001_initial.sql"),
            include_str!("../migrations/0002_import_lifecycle.sql"),
            include_str!("../migrations/0003_canonical_block_kinds.sql"),
            include_str!("../migrations/0004_reader_settings.sql"),
            include_str!("../migrations/0005_provider_profile_summary.sql"),
        ] {
            sqlx::raw_sql(migration).execute(&pool).await.unwrap();
        }

        let profile_id = Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO provider_profiles (id, provider_kind, display_name, model_id, context_window_tokens, credential_key, created_at, updated_at) VALUES (?, 'openai', 'Legacy', 'gpt-4.1-mini', 1000, 'legacy-profile', ?, ?)",
        )
        .bind(&profile_id)
        .bind(common::utc_timestamp())
        .bind(common::utc_timestamp())
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "UPDATE app_settings SET active_provider_profile_id = ?, ui_language = 'legacy-language', font_scale = 1.25, line_height = 1.8, reader_width = 80, pdf_zoom = 1.5 WHERE id = 1",
        )
        .bind(&profile_id)
        .execute(&pool)
        .await
        .unwrap();

        sqlx::raw_sql(include_str!(
            "../migrations/0006_replanned_ui_preferences.sql"
        ))
        .execute(&pool)
        .await
        .unwrap();

        let settings = sqlx::query(
            "SELECT active_provider_profile_id, ui_language, ui_language_initialized, first_reader_hint_completed, font_scale, line_height, reader_width, pdf_zoom FROM app_settings WHERE id = 1",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            settings.get::<Option<String>, _>("active_provider_profile_id"),
            Some(profile_id)
        );
        assert_eq!(settings.get::<String, _>("ui_language"), "en");
        assert_eq!(settings.get::<i64, _>("ui_language_initialized"), 0);
        assert_eq!(settings.get::<i64, _>("first_reader_hint_completed"), 0);
        assert_eq!(settings.get::<f64, _>("font_scale"), 1.25);
        assert_eq!(settings.get::<f64, _>("line_height"), 1.8);
        assert_eq!(settings.get::<f64, _>("reader_width"), 80.0);
        assert_eq!(settings.get::<f64, _>("pdf_zoom"), 1.5);

        let foreign_key_violations: Vec<(String, i64, String, i64)> =
            sqlx::query_as("PRAGMA foreign_key_check")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert!(foreign_key_violations.is_empty());

        let invalid_language =
            sqlx::query("UPDATE app_settings SET ui_language = 'fr' WHERE id = 1")
                .execute(&pool)
                .await;
        assert!(invalid_language.is_err());
    });
}

#[test]
fn provider_preferences_migration_verifies_and_removes_legacy_references() {
    let temp_dir = tempfile::tempdir().unwrap();
    let options = SqliteConnectOptions::new()
        .filename(temp_dir.path().join("provider-preferences.sqlite3"))
        .create_if_missing(true)
        .foreign_keys(true);
    let pool = tauri::async_runtime::block_on(
        SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options),
    )
    .unwrap();

    tauri::async_runtime::block_on(async {
        for migration in [
            include_str!("../migrations/0001_initial.sql"),
            include_str!("../migrations/0002_import_lifecycle.sql"),
            include_str!("../migrations/0003_canonical_block_kinds.sql"),
            include_str!("../migrations/0004_reader_settings.sql"),
            include_str!("../migrations/0005_provider_profile_summary.sql"),
            include_str!("../migrations/0006_replanned_ui_preferences.sql"),
        ] {
            sqlx::raw_sql(migration).execute(&pool).await.unwrap();
        }

        let profile_id = Uuid::new_v4();
        let key = format!("textbooklens/{profile_id}");
        let validated_at = "2026-08-03T03:04:05Z";
        sqlx::query(
            "INSERT INTO provider_profiles (id, provider_kind, display_name, model_id, context_window_tokens, credential_key, is_active, created_at, updated_at, validated_at) VALUES (?, 'openai', 'Legacy OpenAI', 'gpt-5.6', 1050000, ?, 1, ?, ?, ?)",
        )
        .bind(profile_id.to_string())
        .bind(&key)
        .bind(common::utc_timestamp())
        .bind(common::utc_timestamp())
        .bind(validated_at)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("UPDATE app_settings SET active_provider_profile_id = ? WHERE id = 1")
            .bind(profile_id.to_string())
            .execute(&pool)
            .await
            .unwrap();

        sqlx::raw_sql(include_str!(
            "../migrations/0007_provider_operation_preferences.sql"
        ))
        .execute(&pool)
        .await
        .unwrap();

        let columns: Vec<String> = sqlx::query_scalar(
            "SELECT name FROM pragma_table_info('provider_profiles') ORDER BY cid",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert!(!columns.iter().any(|column| column == "credential_key"));
        let profile = sqlx::query(
            "SELECT id, display_name, validated_at, is_active FROM provider_profiles WHERE id = ?",
        )
        .bind(profile_id.to_string())
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(profile.get::<String, _>("id"), profile_id.to_string());
        assert_eq!(profile.get::<String, _>("display_name"), "Legacy OpenAI");
        assert_eq!(profile.get::<String, _>("validated_at"), validated_at);
        assert!(profile.get::<bool, _>("is_active"));

        let settings = sqlx::query(
            "SELECT active_provider_profile_id, default_learning_profile_id, default_vision_profile_id FROM app_settings WHERE id = 1",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            settings.get::<Option<String>, _>("active_provider_profile_id"),
            Some(profile_id.to_string())
        );
        assert_eq!(
            settings.get::<Option<String>, _>("default_learning_profile_id"),
            Some(profile_id.to_string())
        );
        assert_eq!(
            settings.get::<Option<String>, _>("default_vision_profile_id"),
            None
        );

        let consents: Vec<(String, String)> = sqlx::query_as(
            "SELECT category, decision FROM provider_operation_consents WHERE profile_id = ? ORDER BY category",
        )
        .bind(profile_id.to_string())
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(
            consents,
            vec![
                ("ai_index".to_owned(), "ask".to_owned()),
                ("cost_risk".to_owned(), "ask".to_owned()),
                ("image_send".to_owned(), "ask".to_owned()),
            ]
        );
        assert!(
            sqlx::query("UPDATE provider_operation_consents SET category = 'upload' WHERE profile_id = ? AND category = 'image_send'")
                .bind(profile_id.to_string())
                .execute(&pool)
                .await
                .is_err()
        );
        assert!(
            sqlx::query("UPDATE provider_operation_consents SET decision = 'allow' WHERE profile_id = ? AND category = 'image_send'")
                .bind(profile_id.to_string())
                .execute(&pool)
                .await
                .is_err()
        );

        sqlx::query("UPDATE app_settings SET default_vision_profile_id = ? WHERE id = 1")
            .bind(profile_id.to_string())
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM provider_profiles WHERE id = ?")
            .bind(profile_id.to_string())
            .execute(&pool)
            .await
            .unwrap();
        let defaults: (Option<String>, Option<String>, Option<String>) = sqlx::query_as(
            "SELECT active_provider_profile_id, default_learning_profile_id, default_vision_profile_id FROM app_settings WHERE id = 1",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(defaults, (None, None, None));
        let consent_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM provider_operation_consents")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(consent_count, 0);

        let schema: String = sqlx::query_scalar(
            "SELECT group_concat(sql, ' ') FROM sqlite_master WHERE sql IS NOT NULL",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(!schema.contains("credential_key"));
        assert!(!schema.contains(&key));
        let foreign_key_violations: Vec<(String, i64, String, i64)> =
            sqlx::query_as("PRAGMA foreign_key_check")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert!(foreign_key_violations.is_empty());
    });
}

#[test]
fn provider_preferences_migration_aborts_on_a_non_derived_legacy_reference() {
    let temp_dir = tempfile::tempdir().unwrap();
    let options = SqliteConnectOptions::new()
        .filename(temp_dir.path().join("invalid-provider-reference.sqlite3"))
        .create_if_missing(true)
        .foreign_keys(true);
    let pool = tauri::async_runtime::block_on(
        SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options),
    )
    .unwrap();

    tauri::async_runtime::block_on(async {
        for migration in [
            include_str!("../migrations/0001_initial.sql"),
            include_str!("../migrations/0002_import_lifecycle.sql"),
            include_str!("../migrations/0003_canonical_block_kinds.sql"),
            include_str!("../migrations/0004_reader_settings.sql"),
            include_str!("../migrations/0005_provider_profile_summary.sql"),
            include_str!("../migrations/0006_replanned_ui_preferences.sql"),
        ] {
            sqlx::raw_sql(migration).execute(&pool).await.unwrap();
        }
        let profile_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO provider_profiles (id, provider_kind, display_name, model_id, context_window_tokens, credential_key, created_at, updated_at) VALUES (?, 'openai', 'Invalid reference', 'gpt-5.6', 1050000, 'textbooklens/not-the-profile', ?, ?)",
        )
        .bind(profile_id.to_string())
        .bind(common::utc_timestamp())
        .bind(common::utc_timestamp())
        .execute(&pool)
        .await
        .unwrap();

        let mut transaction = pool.begin().await.unwrap();
        let result = sqlx::raw_sql(include_str!(
            "../migrations/0007_provider_operation_preferences.sql"
        ))
        .execute(&mut *transaction)
        .await;
        assert!(result.is_err());
        transaction.rollback().await.unwrap();

        let stored_reference: String =
            sqlx::query_scalar("SELECT credential_key FROM provider_profiles WHERE id = ?")
                .bind(profile_id.to_string())
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(stored_reference, "textbooklens/not-the-profile");
        let new_table_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name IN ('provider_profiles_v7', 'provider_operation_consents')",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(new_table_count, 0);
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

#[test]
fn ai_local_index_migration_forward_preserves_real_v8_content() {
    let temp_dir = tempfile::tempdir().unwrap();
    let database_path = temp_dir.path().join("ai-index-forward.sqlite3");

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
        apply_real_migrations_through_v8(&pool).await;

        let timestamp = "2026-08-04T00:00:00.000Z";
        let book_id = Uuid::new_v4().to_string();
        let section_id = Uuid::new_v4().to_string();
        let block_id = Uuid::new_v4().to_string();
        let chunk_id = Uuid::new_v4().to_string();
        let profile_id = Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO provider_profiles (id, provider_kind, display_name, model_id, context_window_tokens, created_at, updated_at, validated_at) VALUES (?, 'openai', 'Synthetic profile', 'synthetic-model', 32000, ?, ?, ?)",
        )
        .bind(&profile_id)
        .bind(timestamp)
        .bind(timestamp)
        .bind(timestamp)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO books (id, sha256, title, format, original_filename, stored_path, import_status, created_at, updated_at) VALUES (?, ?, 'Synthetic retained book', 'pdf', 'synthetic.pdf', 'books/synthetic/original.pdf', 'ready', ?, ?)",
        )
        .bind(&book_id)
        .bind("a".repeat(64))
        .bind(timestamp)
        .bind(timestamp)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO sections (id, book_id, ordinal, title, locator_json) VALUES (?, ?, 0, 'Synthetic section', '{}')",
        )
        .bind(&section_id)
        .bind(&book_id)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO blocks (id, book_id, section_id, ordinal, kind, plain_text, locator_json) VALUES (?, ?, ?, 0, 'paragraph', 'retained local block', '{}')",
        )
        .bind(&block_id)
        .bind(&book_id)
        .bind(&section_id)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO search_chunks (id, book_id, section_id, ordinal, text, locator_json, token_estimate) VALUES (?, ?, ?, 0, 'retained local search text', '{}', 4)",
        )
        .bind(&chunk_id)
        .bind(&book_id)
        .bind(&section_id)
        .execute(&pool)
        .await
        .unwrap();

        sqlx::raw_sql(include_str!("../migrations/0009_ai_local_index.sql"))
            .execute(&pool)
            .await
            .unwrap();

        assert_eq!(
            sqlx::query_scalar::<_, String>("SELECT plain_text FROM blocks WHERE id = ?")
                .bind(&block_id)
                .fetch_one(&pool)
                .await
                .unwrap(),
            "retained local block"
        );
        assert_eq!(
            sqlx::query_scalar::<_, String>("SELECT text FROM search_chunks WHERE id = ?")
                .bind(&chunk_id)
                .fetch_one(&pool)
                .await
                .unwrap(),
            "retained local search text"
        );
        assert_eq!(
            sqlx::query_scalar::<_, String>(
                "SELECT text FROM search_chunks_fts WHERE search_chunks_fts MATCH 'retained'",
            )
            .fetch_one(&pool)
            .await
            .unwrap(),
            "retained local search text"
        );
        let legacy_search_columns: Vec<String> =
            sqlx::query_scalar("SELECT name FROM pragma_table_info('search_chunks') ORDER BY cid")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(
            legacy_search_columns,
            [
                "id",
                "book_id",
                "section_id",
                "ordinal",
                "text",
                "locator_json",
                "token_estimate",
            ]
        );
        let new_tables: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sqlite_master WHERE type IN ('table', 'view') AND name IN ('index_runs', 'index_pages', 'index_page_blocks', 'index_search_chunks', 'index_search_chunks_fts', 'index_corrections', 'provider_remote_resources')",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(new_tables, 7);
        let foreign_key_violations: Vec<(String, i64, String, i64)> =
            sqlx::query_as("PRAGMA foreign_key_check")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert!(foreign_key_violations.is_empty());
    });
}

#[test]
fn ai_local_index_schema_uses_strict_page_attempt_ownership_and_derived_aggregate() {
    let temp_dir = tempfile::tempdir().unwrap();
    let database = Database::open(temp_dir.path().join("ai-index-contract.sqlite3")).unwrap();

    tauri::async_runtime::block_on(async {
        let fixture = create_ai_index_fixture(database.pool()).await;
        assert_eq!(
            fixture.page_id,
            textbooklens_lib::domain::stable_index_page_id(fixture.run_id, 1)
        );
        assert_eq!(fixture.status, IndexPageStatus::Queued);
        assert!(fixture.attempt_id.is_some());

        let stored_status: String =
            sqlx::query_scalar("SELECT status FROM index_pages WHERE id = ?")
                .bind(fixture.page_id.to_string())
                .fetch_one(database.pool())
                .await
                .unwrap();
        assert_eq!(
            IndexPageStatus::from_database(&stored_status),
            Some(IndexPageStatus::Queued)
        );

        let aggregate = get_run_aggregate(database.pool(), fixture.run_id)
            .await
            .unwrap();
        assert_eq!(aggregate.aggregate_status, IndexAggregateStatus::Partial);
        assert_eq!(aggregate.pages.total, 1);
        assert_eq!(aggregate.pages.queued, 1);

        let reliable_page = create_index_page(
            database.pool(),
            CreateIndexPage {
                run_id: fixture.run_id,
                page_number: 2,
                quality_reason: IndexQualityReason::ReliableText,
                local_text_sha256: Some("b".repeat(64)),
            },
        )
        .await
        .unwrap();
        assert_eq!(reliable_page.status, IndexPageStatus::NotRequired);
        assert_eq!(reliable_page.attempt_id, None);

        let timestamp = "2026-08-04T00:00:00.000Z";
        let invalid_transient = sqlx::query(
            "INSERT INTO index_pages (id, run_id, book_id, page_number, quality_reason, status, attempt_count, created_at, updated_at) VALUES (?, ?, ?, 3, 'no_text', 'rendering', 1, ?, ?)",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(fixture.run_id.to_string())
        .bind(fixture.book_id.to_string())
        .bind(timestamp)
        .bind(timestamp)
        .execute(database.pool())
        .await;
        assert!(invalid_transient.is_err());

        let invalid_status = sqlx::query("UPDATE index_pages SET status = 'rendered' WHERE id = ?")
            .bind(fixture.page_id.to_string())
            .execute(database.pool())
            .await;
        assert!(invalid_status.is_err());

        let duplicate_page = create_index_page(
            database.pool(),
            CreateIndexPage {
                run_id: fixture.run_id,
                page_number: 1,
                quality_reason: IndexQualityReason::NoText,
                local_text_sha256: None,
            },
        )
        .await;
        assert!(duplicate_page.is_err());

        let second_book_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO books (id, sha256, title, format, original_filename, stored_path, import_status, created_at, updated_at) VALUES (?, ?, 'Decoy book', 'pdf', 'decoy.pdf', 'books/decoy/original.pdf', 'ready', ?, ?)",
        )
        .bind(second_book_id.to_string())
        .bind("c".repeat(64))
        .bind(timestamp)
        .bind(timestamp)
        .execute(database.pool())
        .await
        .unwrap();
        let cross_book_page = sqlx::query(
            "INSERT INTO index_pages (id, run_id, book_id, page_number, quality_reason, status, attempt_id, attempt_count, created_at, updated_at) VALUES (?, ?, ?, 4, 'no_text', 'queued', ?, 1, ?, ?)",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(fixture.run_id.to_string())
        .bind(second_book_id.to_string())
        .bind(Uuid::new_v4().to_string())
        .bind(timestamp)
        .bind(timestamp)
        .execute(database.pool())
        .await;
        assert!(cross_book_page.is_err());

        let foreign_key_violations: Vec<(String, i64, String, i64)> =
            sqlx::query_as("PRAGMA foreign_key_check")
                .fetch_all(database.pool())
                .await
                .unwrap();
        assert!(foreign_key_violations.is_empty());
    });
}

#[test]
fn current_index_run_lookup_is_book_scoped_deterministic_and_fails_closed() {
    let temp_dir = tempfile::tempdir().unwrap();
    let database = Database::open(temp_dir.path().join("current-index-run.sqlite3")).unwrap();

    tauri::async_runtime::block_on(async {
        let first = create_ai_index_fixture(database.pool()).await;
        let second = create_ai_index_fixture_with_sha256(database.pool(), "b".repeat(64)).await;
        assert!(
            find_current_run_aggregate_for_book(database.pool(), Uuid::new_v4())
                .await
                .unwrap()
                .is_none()
        );

        let profile_id: String =
            sqlx::query_scalar("SELECT id FROM provider_profiles ORDER BY id LIMIT 1")
                .fetch_one(database.pool())
                .await
                .unwrap();
        let profile_id = Uuid::parse_str(&profile_id).unwrap();
        let later_run = create_index_run(
            database.pool(),
            CreateIndexRun {
                book_id: first.book_id,
                provider_profile_id: profile_id,
                analysis_schema_version: "page-analysis-v1".to_owned(),
                render_version: "pdfjs-render-v1".to_owned(),
                parser_version: "validator-v1".to_owned(),
            },
        )
        .await
        .unwrap();
        let tied_timestamp = "2026-08-04T01:00:00.000Z";
        sqlx::query("UPDATE index_runs SET updated_at = ? WHERE id IN (?, ?)")
            .bind(tied_timestamp)
            .bind(first.run_id.to_string())
            .bind(later_run.to_string())
            .execute(database.pool())
            .await
            .unwrap();

        let first_current = find_current_run_aggregate_for_book(database.pool(), first.book_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(first_current.book_id, first.book_id);
        assert_eq!(
            first_current.run_id,
            if first.run_id > later_run {
                first.run_id
            } else {
                later_run
            }
        );
        let second_current = find_current_run_aggregate_for_book(database.pool(), second.book_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(second_current.book_id, second.book_id);
        assert_eq!(second_current.run_id, second.run_id);

        let corrupt_run = sqlx::query(
            "INSERT INTO index_runs (id, book_id, source_sha256, provider_profile_id, provider_kind, model_id, analysis_schema_version, render_version, parser_version, status, created_at, updated_at) VALUES (?, ?, ?, ?, 'openai', 'synthetic-model', 'page-analysis-v1', 'pdfjs-render-v1', 'validator-v1', 'running', ?, ?)",
        )
        .bind("not-a-canonical-uuid")
        .bind(first.book_id.to_string())
        .bind("d".repeat(64))
        .bind(profile_id.to_string())
        .bind("2026-08-04T02:00:00.000Z")
        .bind("2026-08-04T02:00:00.000Z")
        .execute(database.pool())
        .await;
        assert!(
            corrupt_run.is_err(),
            "the database must reject a malformed run ID before lookup"
        );
    });
}

#[test]
fn ai_local_index_book_delete_requires_cleanup_and_cascades_local_content() {
    let temp_dir = tempfile::tempdir().unwrap();
    let database = Database::open(temp_dir.path().join("ai-index-delete.sqlite3")).unwrap();

    tauri::async_runtime::block_on(async {
        let fixture = create_ai_index_fixture(database.pool()).await;
        let timestamp = "2026-08-04T00:01:00.000Z";
        let attempt_id = fixture.attempt_id.unwrap();
        sqlx::query(
            "UPDATE index_pages SET status = 'needs_review', attempt_started_at = ?, render_sha256 = ?, response_sha256 = ?, content_sha256 = ?, content_version = 1, review_reason_code = 'incomplete_content', updated_at = ? WHERE id = ? AND status = 'queued' AND attempt_id = ?",
        )
        .bind(timestamp)
        .bind("d".repeat(64))
        .bind("e".repeat(64))
        .bind("f".repeat(64))
        .bind(timestamp)
        .bind(fixture.page_id.to_string())
        .bind(attempt_id.to_string())
        .execute(database.pool())
        .await
        .unwrap();

        let block_id =
            stable_index_page_block_id(fixture.book_id, 1, fixture.run_id, "page-analysis-v1", 0);
        sqlx::query(
            "INSERT INTO index_page_blocks (id, page_id, run_id, book_id, ordinal, kind, plain_text, bounds_x, bounds_y, bounds_width, bounds_height, source, provenance_version, content_version, value_sha256, created_at) VALUES (?, ?, ?, ?, 0, 'paragraph', 'synthetic indexed passage', 0.1, 0.1, 0.8, 0.2, 'ai_transcribed', 1, 1, ?, ?)",
        )
        .bind(block_id.to_string())
        .bind(fixture.page_id.to_string())
        .bind(fixture.run_id.to_string())
        .bind(fixture.book_id.to_string())
        .bind("1".repeat(64))
        .bind(timestamp)
        .execute(database.pool())
        .await
        .unwrap();

        let correction_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO index_corrections (id, book_id, page_id, target_block_id, target_content_version, value_kind, original_value_sha256, original_value, corrected_value, conflict_state, created_at, updated_at) VALUES (?, ?, ?, ?, 1, 'text', ?, 'synthetic indexed passage', 'synthetic corrected passage', 'active', ?, ?)",
        )
        .bind(correction_id.to_string())
        .bind(fixture.book_id.to_string())
        .bind(fixture.page_id.to_string())
        .bind(block_id.to_string())
        .bind("1".repeat(64))
        .bind(timestamp)
        .bind(timestamp)
        .execute(database.pool())
        .await
        .unwrap();

        let chunk_id = stable_index_search_chunk_id(block_id, ContentSource::AiTranscribed, 0);
        sqlx::query(
            "INSERT INTO index_search_chunks (id, book_id, page_id, block_id, ordinal, source, text, locator_json, token_estimate, content_version, created_at) VALUES (?, ?, ?, ?, 0, 'ai_transcribed', 'synthetic indexed passage', ?, 4, 1, ?)",
        )
        .bind(chunk_id.to_string())
        .bind(fixture.book_id.to_string())
        .bind(fixture.page_id.to_string())
        .bind(block_id.to_string())
        .bind(r#"{"format":"pdf","startPage":1,"endPage":1,"rectsByPage":null}"#)
        .bind(timestamp)
        .execute(database.pool())
        .await
        .unwrap();
        let indexed_fts_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM index_search_chunks_fts WHERE index_search_chunks_fts MATCH 'synthetic'",
        )
        .fetch_one(database.pool())
        .await
        .unwrap();
        assert_eq!(indexed_fts_count, 1);

        let remote_resource_id = track_remote_resource(
            database.pool(),
            fixture.page_id,
            EncryptedRemoteResourceReference::new("enc:v1:synthetic-envelope".to_owned()).unwrap(),
        )
        .await
        .unwrap();

        let premature_delete = sqlx::query("DELETE FROM books WHERE id = ?")
            .bind(fixture.book_id.to_string())
            .execute(database.pool())
            .await;
        assert!(premature_delete.is_err());
        let premature_resource_drop =
            sqlx::query("DELETE FROM provider_remote_resources WHERE id = ?")
                .bind(remote_resource_id.to_string())
                .execute(database.pool())
                .await;
        assert!(premature_resource_drop.is_err());

        let cleanup_attempt_id = Uuid::new_v4();
        sqlx::query(
            "UPDATE provider_remote_resources SET cleanup_status = 'cleaning', cleanup_attempt_id = ?, cleanup_attempt_count = 1, last_cleanup_at = ?, updated_at = ? WHERE id = ? AND cleanup_status = 'pending'",
        )
        .bind(cleanup_attempt_id.to_string())
        .bind(timestamp)
        .bind(timestamp)
        .bind(remote_resource_id.to_string())
        .execute(database.pool())
        .await
        .unwrap();
        sqlx::query(
            "UPDATE provider_remote_resources SET cleanup_status = 'failed', safe_error_code = 'REMOTE_DELETE_FAILED', safe_error_message = 'Remote cleanup did not complete.', updated_at = ? WHERE id = ? AND cleanup_status = 'cleaning' AND cleanup_attempt_id = ?",
        )
        .bind(timestamp)
        .bind(remote_resource_id.to_string())
        .bind(cleanup_attempt_id.to_string())
        .execute(database.pool())
        .await
        .unwrap();

        sqlx::query("DELETE FROM books WHERE id = ?")
            .bind(fixture.book_id.to_string())
            .execute(database.pool())
            .await
            .unwrap();

        for (table, count_query) in [
            ("index_runs", "SELECT COUNT(*) FROM index_runs"),
            ("index_pages", "SELECT COUNT(*) FROM index_pages"),
            (
                "index_page_blocks",
                "SELECT COUNT(*) FROM index_page_blocks",
            ),
            (
                "index_search_chunks",
                "SELECT COUNT(*) FROM index_search_chunks",
            ),
            (
                "index_corrections",
                "SELECT COUNT(*) FROM index_corrections",
            ),
        ] {
            let count: i64 = sqlx::query_scalar(count_query)
                .fetch_one(database.pool())
                .await
                .unwrap();
            assert_eq!(count, 0, "{table} must cascade with the book");
        }
        let deleted_fts_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM index_search_chunks_fts WHERE index_search_chunks_fts MATCH 'synthetic'",
        )
        .fetch_one(database.pool())
        .await
        .unwrap();
        assert_eq!(deleted_fts_count, 0);

        let retained_remote = sqlx::query(
            "SELECT book_id, run_id, page_id, cleanup_status FROM provider_remote_resources WHERE id = ?",
        )
        .bind(remote_resource_id.to_string())
        .fetch_one(database.pool())
        .await
        .unwrap();
        assert_eq!(retained_remote.get::<Option<String>, _>("book_id"), None);
        assert_eq!(retained_remote.get::<Option<String>, _>("run_id"), None);
        assert_eq!(retained_remote.get::<Option<String>, _>("page_id"), None);
        assert_eq!(retained_remote.get::<String, _>("cleanup_status"), "failed");

        let unresolved_drop = sqlx::query("DELETE FROM provider_remote_resources WHERE id = ?")
            .bind(remote_resource_id.to_string())
            .execute(database.pool())
            .await;
        assert!(unresolved_drop.is_err());
        sqlx::query(
            "UPDATE provider_remote_resources SET cleanup_status = 'safely_disposed', safe_error_code = NULL, safe_error_message = NULL, disposition_reason = 'provider_expired', updated_at = ? WHERE id = ? AND cleanup_status = 'failed' AND cleanup_attempt_id = ?",
        )
        .bind(timestamp)
        .bind(remote_resource_id.to_string())
        .bind(cleanup_attempt_id.to_string())
        .execute(database.pool())
        .await
        .unwrap();
        sqlx::query("DELETE FROM provider_remote_resources WHERE id = ?")
            .bind(remote_resource_id.to_string())
            .execute(database.pool())
            .await
            .unwrap();
    });
}

async fn apply_real_migrations_through_v8(pool: &sqlx::SqlitePool) {
    for migration in [
        include_str!("../migrations/0001_initial.sql"),
        include_str!("../migrations/0002_import_lifecycle.sql"),
        include_str!("../migrations/0003_canonical_block_kinds.sql"),
        include_str!("../migrations/0004_reader_settings.sql"),
        include_str!("../migrations/0005_provider_profile_summary.sql"),
        include_str!("../migrations/0006_replanned_ui_preferences.sql"),
        include_str!("../migrations/0007_provider_operation_preferences.sql"),
        include_str!("../migrations/0008_teaching_instruction.sql"),
    ] {
        sqlx::raw_sql(migration).execute(pool).await.unwrap();
    }
}

async fn apply_real_migrations_through_v10(pool: &sqlx::SqlitePool) {
    apply_real_migrations_through_v8(pool).await;
    for migration in [
        include_str!("../migrations/0009_ai_local_index.sql"),
        include_str!("../migrations/0010_note_revision.sql"),
    ] {
        sqlx::raw_sql(migration).execute(pool).await.unwrap();
    }
}

struct AiIndexFixture {
    book_id: Uuid,
    run_id: Uuid,
    page_id: Uuid,
    attempt_id: Option<Uuid>,
    status: IndexPageStatus,
}

async fn create_ai_index_fixture(pool: &sqlx::SqlitePool) -> AiIndexFixture {
    create_ai_index_fixture_with_sha256(pool, "a".repeat(64)).await
}

async fn create_ai_index_fixture_with_sha256(
    pool: &sqlx::SqlitePool,
    sha256: String,
) -> AiIndexFixture {
    let timestamp = "2026-08-04T00:00:00.000Z";
    let book_id = Uuid::new_v4();
    let profile_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO books (id, sha256, title, format, original_filename, stored_path, import_status, created_at, updated_at) VALUES (?, ?, 'Synthetic AI index book', 'pdf', 'synthetic.pdf', 'books/synthetic/original.pdf', 'ready', ?, ?)",
    )
    .bind(book_id.to_string())
    .bind(sha256)
    .bind(timestamp)
    .bind(timestamp)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO provider_profiles (id, provider_kind, display_name, model_id, context_window_tokens, created_at, updated_at, validated_at) VALUES (?, 'openai', 'Synthetic index profile', 'synthetic-model', 32000, ?, ?, ?)",
    )
    .bind(profile_id.to_string())
    .bind(timestamp)
    .bind(timestamp)
    .bind(timestamp)
    .execute(pool)
    .await
    .unwrap();

    let run_id = create_index_run(
        pool,
        CreateIndexRun {
            book_id,
            provider_profile_id: profile_id,
            analysis_schema_version: "page-analysis-v1".to_owned(),
            render_version: "pdfjs-render-v1".to_owned(),
            parser_version: "validator-v1".to_owned(),
        },
    )
    .await
    .unwrap();
    let page = create_index_page(
        pool,
        CreateIndexPage {
            run_id,
            page_number: 1,
            quality_reason: IndexQualityReason::NoText,
            local_text_sha256: None,
        },
    )
    .await
    .unwrap();

    AiIndexFixture {
        book_id,
        run_id,
        page_id: page.page_id,
        attempt_id: page.attempt_id,
        status: page.status,
    }
}
