use std::{fs, path::Path};

#[cfg(windows)]
use std::process::{Command, Stdio};

use sha2::{Digest, Sha256};
use sqlx::{
    Row, SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use tempfile::TempDir;
use textbooklens_lib::{
    app_state::AppPaths,
    db::{Database, MIGRATOR},
    errors::{AppErrorCode, AppErrorDto},
    maintenance::{
        delete_book::recover_pending_deletions_async,
        journal::{DeleteJournal, JournalStore, NoJournalFault, RelativePathToken},
        storage::{APP_DATA_DIRECTORY_NAME, prepare_app_data_paths},
    },
};
use uuid::Uuid;

const MIGRATION_HASHES: &[(i64, &str, &[u8], &str)] = &[
    (
        1,
        "0001_initial.sql",
        include_bytes!("../migrations/0001_initial.sql"),
        "24a177f2d1c63d377e8771b9c95674c90d68fc0e8b02aed3db0cdc1a131eda8b",
    ),
    (
        2,
        "0002_import_lifecycle.sql",
        include_bytes!("../migrations/0002_import_lifecycle.sql"),
        "586469b9cb3ab148c4730681830813be278842ee3777cf74cb9030fd0b023aff",
    ),
    (
        3,
        "0003_canonical_block_kinds.sql",
        include_bytes!("../migrations/0003_canonical_block_kinds.sql"),
        "ebfb8935c1e8505411a4fcbd9d4ca4986817b2d8357e4ae506d91beda3f2c62b",
    ),
    (
        4,
        "0004_reader_settings.sql",
        include_bytes!("../migrations/0004_reader_settings.sql"),
        "82a81e70944a7944d0375d140d9bf7c16ec0754bfd71a341136c377c73da4e63",
    ),
    (
        5,
        "0005_provider_profile_summary.sql",
        include_bytes!("../migrations/0005_provider_profile_summary.sql"),
        "8365e58ee50adac50ba75c8b13a9e7070c41cb1d020b8091308a6838a2270ab6",
    ),
    (
        6,
        "0006_replanned_ui_preferences.sql",
        include_bytes!("../migrations/0006_replanned_ui_preferences.sql"),
        "59818a0da0273ff78d1341d8012d48f839ad49b90c3264c4e7ac6c8c4f3b597d",
    ),
    (
        7,
        "0007_provider_operation_preferences.sql",
        include_bytes!("../migrations/0007_provider_operation_preferences.sql"),
        "2d8eaa2aec27c6ab085d6ec08ed02627621c960f4249aa748a9564063952be33",
    ),
    (
        8,
        "0008_teaching_instruction.sql",
        include_bytes!("../migrations/0008_teaching_instruction.sql"),
        "c27442cb057b05060131259ecf4b1ae3667db457d24df710613a8b6ee7f53398",
    ),
    (
        9,
        "0009_ai_local_index.sql",
        include_bytes!("../migrations/0009_ai_local_index.sql"),
        "1f035cf57442a1133d3fe9a17934c18c73de9087e385637b824cfaf92111b6c2",
    ),
    (
        10,
        "0010_note_revision.sql",
        include_bytes!("../migrations/0010_note_revision.sql"),
        "84e5670dc40cda076a0fda41ac2b9a1bbb37883d525d937d0b87cccb349360a2",
    ),
    (
        11,
        "0011_learning_panels.sql",
        include_bytes!("../migrations/0011_learning_panels.sql"),
        "bc92621c832409a17c7838d30aa3465c5b8eff198d7aa7e5d8c0baf3cb295002",
    ),
    (
        12,
        "0012_kimi_file_extraction.sql",
        include_bytes!("../migrations/0012_kimi_file_extraction.sql"),
        "f3b1c82e74c9defca104017046e0a996ed5a25f9ae63c98087454a53c1d17905",
    ),
    (
        13,
        "0013_annotation_summaries.sql",
        include_bytes!("../migrations/0013_annotation_summaries.sql"),
        "e1634648f547d59241470434374ec4a8f151fe14daa0bd5648f1eebf776213c5",
    ),
    (
        14,
        "0014_filename_title_fallback.sql",
        include_bytes!("../migrations/0014_filename_title_fallback.sql"),
        "a65456b829b59e644ff3e6382cf12af0eb91a8e7545cca1a6e4f269f3b15f574",
    ),
    (
        15,
        "0015_kimi_api_region_binding.sql",
        include_bytes!("../migrations/0015_kimi_api_region_binding.sql"),
        "55556c7e0824176396be3798b28263bc8f0441c4603cf43098c60c1742effd75",
    ),
];

const BOOK_ID: &str = "00000000-0000-4000-8000-000000000101";
const SECTION_ID: &str = "00000000-0000-4000-8000-000000000102";
const BLOCK_ID: &str = "00000000-0000-4000-8000-000000000103";
const CHUNK_ID: &str = "00000000-0000-4000-8000-000000000104";
const CONVERSATION_ID: &str = "00000000-0000-4000-8000-000000000105";
const ANNOTATION_ID: &str = "00000000-0000-4000-8000-000000000106";
const USER_MESSAGE_ID: &str = "00000000-0000-4000-8000-000000000107";
const ASSISTANT_MESSAGE_ID: &str = "00000000-0000-4000-8000-000000000108";
const PROFILE_ID: &str = "00000000-0000-4000-8000-000000000109";

type SchemaRow = (String, String, String, Option<String>);

#[test]
fn migration_files_and_embedded_history_are_immutable_through_v15() {
    assert_eq!(MIGRATOR.migrations.len(), MIGRATION_HASHES.len());
    assert_eq!(
        MIGRATOR
            .migrations
            .iter()
            .filter(|migration| migration.no_tx)
            .map(|migration| migration.version)
            .collect::<Vec<_>>(),
        vec![2, 11]
    );

    for ((version, name, bytes, expected_hash), migration) in
        MIGRATION_HASHES.iter().zip(MIGRATOR.migrations.iter())
    {
        assert_eq!(
            migration.version, *version,
            "migration version drift: {name}"
        );
        let expected_description = name
            .trim_end_matches(".sql")
            .split_once('_')
            .unwrap()
            .1
            .replace('_', " ");
        assert_eq!(
            migration.description.as_ref(),
            expected_description,
            "migration description drift: {name}"
        );
        assert_eq!(
            hex(Sha256::digest(bytes)),
            *expected_hash,
            "migration content drift: {name}"
        );
    }
}

#[test]
fn every_historical_cutoff_upgrades_to_the_exact_fresh_contract_and_reopens() {
    let fresh = TempDir::new().unwrap();
    let fresh_path = fresh.path().join("fresh.sqlite3");
    let fresh_database = Database::open(&fresh_path).unwrap();
    let expected_schema = block_on(schema_rows(fresh_database.pool()));
    block_on(assert_current_contract(fresh_database.pool(), false));
    block_on(fresh_database.pool().close());
    drop(fresh_database);

    for cutoff in 1..=15 {
        let temporary = TempDir::new().unwrap();
        let database_path = temporary.path().join(format!("cutoff-{cutoff}.sqlite3"));
        let pool = block_on(raw_pool(&database_path));
        block_on(MIGRATOR.run_to(cutoff, &pool)).unwrap();
        block_on(seed_historical_fixture(&pool, cutoff));
        block_on(pool.close());

        let upgraded = Database::open(&database_path).unwrap();
        assert_eq!(block_on(schema_rows(upgraded.pool())), expected_schema);
        block_on(assert_current_contract(upgraded.pool(), true));
        block_on(upgraded.pool().close());
        drop(upgraded);

        let reopened = Database::open(&database_path).unwrap();
        assert_eq!(block_on(schema_rows(reopened.pool())), expected_schema);
        block_on(assert_current_contract(reopened.pool(), true));
        block_on(reopened.pool().close());
    }
}

#[test]
fn malformed_migration_metadata_and_user_version_fail_before_schema_mutation() {
    user_version_attack_is_rejected();
    disguised_non_internal_schema_without_history_is_rejected();
    missing_history_table_is_rejected_without_recreation();
    duplicate_history_is_rejected();
    noncontiguous_history_is_rejected_without_partial_migration();
    late_migration_failure_rolls_back_schema_and_history();
    checksum_and_dirty_history_are_rejected();
}

fn disguised_non_internal_schema_without_history_is_rejected() {
    let temporary = TempDir::new().unwrap();
    let path = temporary
        .path()
        .join("private-path-marker-disguised-schema.sqlite3");
    let pool = block_on(raw_pool(&path));
    block_on(sqlx::query("CREATE TABLE sqlitex_decoy (id INTEGER)").execute(&pool)).unwrap();
    let before = block_on(schema_rows(&pool));
    block_on(pool.close());

    assert_safe_open_rejection(&path);
    let pool = block_on(raw_pool(&path));
    assert_eq!(block_on(schema_rows(&pool)), before);
    assert!(block_on(object_exists(&pool, "sqlitex_decoy")));
    assert!(!block_on(object_exists(&pool, "_sqlx_migrations")));
    block_on(pool.close());
}

#[test]
fn app_root_preflight_rejects_hardlinked_database_and_path_alias_attacks() {
    let temporary = TempDir::new().unwrap();
    let root = temporary.path().join(APP_DATA_DIRECTORY_NAME);
    fs::create_dir(&root).unwrap();
    let outside = temporary.path().join("outside.sqlite3");
    fs::write(&outside, b"external-decoy").unwrap();
    fs::hard_link(&outside, root.join("library.sqlite3")).unwrap();

    let error = prepare_app_data_paths(&root).unwrap_err();
    assert_eq!(
        format!("{error:?}"),
        "StorageError { code: StorageRootInvalid }"
    );
    assert_eq!(fs::read(&outside).unwrap(), b"external-decoy");

    for candidate in [
        Path::new(APP_DATA_DIRECTORY_NAME).to_path_buf(),
        temporary.path().join("..").join(APP_DATA_DIRECTORY_NAME),
        temporary.path().join(format!("{APP_DATA_DIRECTORY_NAME}*")),
        temporary
            .path()
            .join(format!("{APP_DATA_DIRECTORY_NAME}%TEMP%")),
        temporary
            .path()
            .join(format!("{APP_DATA_DIRECTORY_NAME}$env:TEMP")),
        temporary
            .path()
            .join(APP_DATA_DIRECTORY_NAME.to_ascii_uppercase()),
        temporary.path().to_path_buf(),
        #[cfg(windows)]
        Path::new(r"\\server\share\dev.textbooklens.desktop").to_path_buf(),
        #[cfg(windows)]
        Path::new(r"\\?\C:\private\dev.textbooklens.desktop").to_path_buf(),
        #[cfg(windows)]
        Path::new(r"C:\").to_path_buf(),
    ] {
        let error = prepare_app_data_paths(&candidate).unwrap_err();
        assert_eq!(
            format!("{error:?}"),
            "StorageError { code: StorageRootInvalid }"
        );
    }

    #[cfg(windows)]
    {
        let outside_parent = temporary.path().join("junction-target");
        let alias_parent = temporary.path().join("junction-alias");
        fs::create_dir(&outside_parent).unwrap();
        fs::create_dir(outside_parent.join(APP_DATA_DIRECTORY_NAME)).unwrap();
        fs::write(outside_parent.join("external-decoy.bin"), b"junction-decoy").unwrap();
        create_windows_junction(&outside_parent, &alias_parent);
        let alias_root = alias_parent.join(APP_DATA_DIRECTORY_NAME);
        let error = prepare_app_data_paths(&alias_root).unwrap_err();
        assert_eq!(
            format!("{error:?}"),
            "StorageError { code: StorageRootInvalid }"
        );
        assert_eq!(
            fs::read(outside_parent.join("external-decoy.bin")).unwrap(),
            b"junction-decoy"
        );
    }
}

#[test]
fn all_delete_journals_are_preflighted_before_any_prior_plan_can_mutate() {
    let temporary = TempDir::new().unwrap();
    let prepared = prepare_app_data_paths(&temporary.path().join(APP_DATA_DIRECTORY_NAME)).unwrap();
    let paths = AppPaths {
        root: prepared.root,
        books: prepared.books,
        cache: prepared.cache,
        logs: prepared.logs,
        database: prepared.database,
    };
    let database = Database::open(&paths.database).unwrap();
    let first_book = Uuid::from_u128(0x201);
    let second_book = Uuid::from_u128(0x202);
    let first_directory = paths.books.join(first_book.to_string());
    let second_directory = paths.books.join(second_book.to_string());
    fs::create_dir(&first_directory).unwrap();
    fs::create_dir(&second_directory).unwrap();
    let first_file = first_directory.join("original.pdf");
    let second_file = second_directory.join("original.pdf");
    fs::write(&first_file, b"shared-synthetic-inode").unwrap();
    fs::hard_link(&first_file, &second_file).unwrap();

    let first_journal = DeleteJournal::from_sources(
        Uuid::from_u128(0x301),
        first_book,
        vec![RelativePathToken::new(format!("books/{first_book}")).unwrap()],
    )
    .unwrap();
    let second_journal = DeleteJournal::from_sources(
        Uuid::from_u128(0x302),
        second_book,
        vec![RelativePathToken::new(format!("books/{second_book}")).unwrap()],
    )
    .unwrap();
    let store = JournalStore::open(&paths).unwrap();
    store.write_intent(&first_journal, &NoJournalFault).unwrap();
    store
        .write_intent(&second_journal, &NoJournalFault)
        .unwrap();

    for _ in 0..2 {
        let error = block_on(recover_pending_deletions_async(database.pool(), &paths)).unwrap_err();
        assert_eq!(error.code, AppErrorCode::InvalidInput);
        let debug = format!("{error:?}");
        assert!(!debug.contains(&first_book.to_string()));
        assert!(!debug.contains(&second_book.to_string()));
        assert_eq!(fs::read(&first_file).unwrap(), b"shared-synthetic-inode");
        assert_eq!(fs::read(&second_file).unwrap(), b"shared-synthetic-inode");
        assert_eq!(
            store.list_intents().unwrap(),
            vec![first_journal.clone(), second_journal.clone()]
        );
    }
    block_on(database.pool().close());
}

#[cfg(windows)]
#[test]
fn windows_exclusive_lock_and_read_only_upgrade_fail_with_recoverable_old_state() {
    let temporary = TempDir::new().unwrap();
    let path = temporary
        .path()
        .join("private-path-marker-windows-lock.sqlite3");
    let pool = block_on(raw_pool(&path));
    block_on(MIGRATOR.run_to(10, &pool)).unwrap();
    block_on(pool.close());

    let lock = ExclusiveWindowsFileLock::acquire(&path);
    assert_safe_open_rejection(&path);
    drop(lock);
    assert_cutoff_ten_state(&path);

    set_windows_readonly(&path, true);
    assert_safe_open_rejection(&path);
    set_windows_readonly(&path, false);
    assert_cutoff_ten_state(&path);

    let upgraded = Database::open(&path).unwrap();
    block_on(assert_current_contract(upgraded.pool(), false));
    block_on(upgraded.pool().close());
}

#[cfg(windows)]
fn assert_cutoff_ten_state(path: &Path) {
    let pool = block_on(raw_pool(path));
    let versions: Vec<i64> = block_on(
        sqlx::query_scalar("SELECT version FROM _sqlx_migrations ORDER BY version")
            .fetch_all(&pool),
    )
    .unwrap();
    assert_eq!(versions, (1..=10).collect::<Vec<_>>());
    assert!(!block_on(object_exists(&pool, "panel_preferences")));
    block_on(pool.close());
}

fn user_version_attack_is_rejected() {
    let temporary = TempDir::new().unwrap();
    let path = temporary
        .path()
        .join("private-path-marker-user-version.sqlite3");
    create_current_database(&path);
    let pool = block_on(raw_pool(&path));
    block_on(sqlx::query("PRAGMA user_version = 2147483647").execute(&pool)).unwrap();
    block_on(pool.close());

    assert_safe_open_rejection(&path);
    let pool = block_on(raw_pool(&path));
    let version: i64 =
        block_on(sqlx::query_scalar("PRAGMA user_version").fetch_one(&pool)).unwrap();
    assert_eq!(version, 2_147_483_647);
    block_on(pool.close());
}

fn missing_history_table_is_rejected_without_recreation() {
    let temporary = TempDir::new().unwrap();
    let path = temporary.path().join("private-path-marker-missing.sqlite3");
    create_current_database(&path);
    let pool = block_on(raw_pool(&path));
    block_on(sqlx::query("DROP TABLE _sqlx_migrations").execute(&pool)).unwrap();
    let before = block_on(schema_rows(&pool));
    block_on(pool.close());

    assert_safe_open_rejection(&path);
    let pool = block_on(raw_pool(&path));
    assert_eq!(block_on(schema_rows(&pool)), before);
    assert!(!block_on(object_exists(&pool, "_sqlx_migrations")));
    block_on(pool.close());
}

fn duplicate_history_is_rejected() {
    let temporary = TempDir::new().unwrap();
    let path = temporary
        .path()
        .join("private-path-marker-duplicate.sqlite3");
    create_current_database(&path);
    let pool = block_on(raw_pool(&path));
    block_on(
        sqlx::raw_sql(
            "ALTER TABLE _sqlx_migrations RENAME TO migration_history_original;
         CREATE TABLE _sqlx_migrations (
           version BIGINT,
           description TEXT NOT NULL,
           installed_on TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
           success BOOLEAN NOT NULL,
           checksum BLOB NOT NULL,
           execution_time BIGINT NOT NULL
         );
         INSERT INTO _sqlx_migrations SELECT * FROM migration_history_original;
         INSERT INTO _sqlx_migrations SELECT * FROM migration_history_original WHERE version = 1;
         DROP TABLE migration_history_original;",
        )
        .execute(&pool),
    )
    .unwrap();
    let before = block_on(schema_rows(&pool));
    block_on(pool.close());

    assert_safe_open_rejection(&path);
    let pool = block_on(raw_pool(&path));
    assert_eq!(block_on(schema_rows(&pool)), before);
    let duplicates: i64 = block_on(
        sqlx::query_scalar("SELECT COUNT(*) FROM _sqlx_migrations WHERE version = 1")
            .fetch_one(&pool),
    )
    .unwrap();
    assert_eq!(duplicates, 2);
    block_on(pool.close());
}

fn noncontiguous_history_is_rejected_without_partial_migration() {
    let temporary = TempDir::new().unwrap();
    let path = temporary.path().join("private-path-marker-gap.sqlite3");
    let pool = block_on(raw_pool(&path));
    block_on(create_migration_history_table(&pool));
    for migration in [
        MIGRATOR
            .migrations
            .iter()
            .find(|migration| migration.version == 1)
            .unwrap(),
        MIGRATOR
            .migrations
            .iter()
            .find(|migration| migration.version == 3)
            .unwrap(),
    ] {
        block_on(insert_migration_row(&pool, migration)).unwrap();
    }
    let before = block_on(schema_rows(&pool));
    block_on(pool.close());

    assert_safe_open_rejection(&path);
    let pool = block_on(raw_pool(&path));
    assert_eq!(block_on(schema_rows(&pool)), before);
    assert!(!block_on(object_exists(&pool, "books_phase2")));
    block_on(pool.close());
}

fn late_migration_failure_rolls_back_schema_and_history() {
    let temporary = TempDir::new().unwrap();
    let path = temporary
        .path()
        .join("private-path-marker-atomic-rollback.sqlite3");
    let pool = block_on(raw_pool(&path));
    block_on(MIGRATOR.run_to(1, &pool)).unwrap();
    let original_sql: String = block_on(
        sqlx::query_scalar("SELECT sql FROM sqlite_schema WHERE type = 'table' AND name = 'books'")
            .fetch_one(&pool),
    )
    .unwrap();
    let weakened_sql = original_sql.replace(
        "format TEXT NOT NULL CHECK (format IN ('pdf', 'epub', 'docx'))",
        "format TEXT NOT NULL",
    );
    assert_ne!(weakened_sql, original_sql);
    block_on(sqlx::query("PRAGMA writable_schema = ON").execute(&pool)).unwrap();
    block_on(
        sqlx::query("UPDATE sqlite_schema SET sql = ? WHERE type = 'table' AND name = 'books'")
            .bind(&weakened_sql)
            .execute(&pool),
    )
    .unwrap();
    let schema_version: i64 =
        block_on(sqlx::query_scalar("PRAGMA schema_version").fetch_one(&pool)).unwrap();
    let set_schema_version = format!("PRAGMA schema_version = {}", schema_version + 1);
    block_on(sqlx::query(sqlx::AssertSqlSafe(set_schema_version)).execute(&pool)).unwrap();
    block_on(sqlx::query("PRAGMA writable_schema = OFF").execute(&pool)).unwrap();
    block_on(pool.close());

    let pool = block_on(raw_pool(&path));
    block_on(
        sqlx::query(
            "INSERT INTO books (
           id, sha256, title, format, original_filename, stored_path, import_status,
           created_at, updated_at
         ) VALUES (?, ?, 'invalid-shape', 'invalid-format', 'invalid.bin', ?, 'ready', ?, ?)",
        )
        .bind(Uuid::from_u128(0x401).to_string())
        .bind("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")
        .bind("books/00000000-0000-4000-8000-000000000401/original.bin")
        .bind("2026-01-01T00:00:00.000Z")
        .bind("2026-01-01T00:00:00.000Z")
        .execute(&pool),
    )
    .unwrap();
    let before = block_on(schema_rows(&pool));
    block_on(pool.close());

    assert_safe_open_rejection(&path);
    let pool = block_on(raw_pool(&path));
    assert_eq!(block_on(schema_rows(&pool)), before);
    assert!(!block_on(object_exists(&pool, "books_phase2")));
    let versions: Vec<i64> = block_on(
        sqlx::query_scalar("SELECT version FROM _sqlx_migrations ORDER BY version")
            .fetch_all(&pool),
    )
    .unwrap();
    assert_eq!(versions, vec![1]);
    let invalid_row: i64 = block_on(
        sqlx::query_scalar("SELECT COUNT(*) FROM books WHERE format = 'invalid-format'")
            .fetch_one(&pool),
    )
    .unwrap();
    assert_eq!(invalid_row, 1);
    block_on(pool.close());
}

fn checksum_and_dirty_history_are_rejected() {
    for mutation in ["checksum", "dirty", "gap"] {
        let temporary = TempDir::new().unwrap();
        let path = temporary
            .path()
            .join(format!("private-path-marker-{mutation}.sqlite3"));
        create_current_database(&path);
        let pool = block_on(raw_pool(&path));
        match mutation {
            "checksum" => {
                block_on(
                    sqlx::query("UPDATE _sqlx_migrations SET checksum = X'00' WHERE version = 4")
                        .execute(&pool),
                )
                .unwrap();
            }
            "dirty" => {
                block_on(
                    sqlx::query("UPDATE _sqlx_migrations SET success = 0 WHERE version = 4")
                        .execute(&pool),
                )
                .unwrap();
            }
            "gap" => {
                block_on(
                    sqlx::query("DELETE FROM _sqlx_migrations WHERE version = 4").execute(&pool),
                )
                .unwrap();
            }
            _ => unreachable!(),
        }
        let before = block_on(schema_rows(&pool));
        block_on(pool.close());

        assert_safe_open_rejection(&path);
        let pool = block_on(raw_pool(&path));
        assert_eq!(block_on(schema_rows(&pool)), before);
        block_on(pool.close());
    }
}

fn create_current_database(path: &Path) {
    let database = Database::open(path).unwrap();
    block_on(database.pool().close());
}

fn assert_safe_open_rejection(path: &Path) {
    let error = match Database::open(path) {
        Ok(database) => {
            block_on(database.pool().close());
            panic!("malformed database was accepted")
        }
        Err(error) => error,
    };
    assert_eq!(error.code, AppErrorCode::DatabaseError);
    let debug = format!("{error:?}");
    let display = error.to_string();
    let dto = serde_json::to_string(&AppErrorDto::from(error)).unwrap();
    for forbidden in [
        "private-path-marker",
        "provider",
        "credential",
        "00000000-0000-4000-8000-000000000109",
    ] {
        assert!(!debug.to_ascii_lowercase().contains(forbidden));
        assert!(!display.to_ascii_lowercase().contains(forbidden));
        assert!(!dto.to_ascii_lowercase().contains(forbidden));
    }
}

async fn raw_pool(path: &Path) -> SqlitePool {
    SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            SqliteConnectOptions::new()
                .filename(path)
                .create_if_missing(true)
                .foreign_keys(true),
        )
        .await
        .unwrap()
}

async fn seed_historical_fixture(pool: &SqlitePool, cutoff: i64) {
    sqlx::query(
        "INSERT INTO books (
           id, sha256, title, author, language, format, original_filename, stored_path,
           import_status, created_at, updated_at, reading_progress
         ) VALUES (?, ?, 'fixture-title', NULL, 'en', 'pdf', 'fixture.pdf', ?, 'ready', ?, ?, 0.25)",
    )
    .bind(BOOK_ID)
    .bind("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
    .bind(format!("books/{BOOK_ID}/original.pdf"))
    .bind("2026-01-01T00:00:00.000Z")
    .bind("2026-01-01T00:00:00.000Z")
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO sections (id, book_id, ordinal, title, locator_json)
         VALUES (?, ?, 0, 'fixture-section', '{}')",
    )
    .bind(SECTION_ID)
    .bind(BOOK_ID)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO blocks (id, book_id, section_id, ordinal, kind, plain_text, locator_json)
         VALUES (?, ?, ?, 0, 'paragraph', 'fixture-block', '{}')",
    )
    .bind(BLOCK_ID)
    .bind(BOOK_ID)
    .bind(SECTION_ID)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO search_chunks (
           id, book_id, section_id, ordinal, text, locator_json, token_estimate
         ) VALUES (?, ?, ?, 0, 'migrationfixtureterm', '{}', 1)",
    )
    .bind(CHUNK_ID)
    .bind(BOOK_ID)
    .bind(SECTION_ID)
    .execute(pool)
    .await
    .unwrap();
    if cutoff >= 11 {
        sqlx::query(
            "INSERT INTO conversations (
               id, book_id, section_id, scope, anchor_kind, anchor_json, selected_text,
               created_at, updated_at
             ) VALUES (?, ?, ?, 'selection', 'text', '{}', 'fixture-selection', ?, ?)",
        )
        .bind(CONVERSATION_ID)
        .bind(BOOK_ID)
        .bind(SECTION_ID)
        .bind("2026-01-01T00:00:00.000Z")
        .bind("2026-01-01T00:00:00.000Z")
        .execute(pool)
        .await
        .unwrap();
    } else {
        sqlx::query(
            "INSERT INTO conversations (
               id, book_id, section_id, scope, anchor_json, selected_text, created_at, updated_at
             ) VALUES (?, ?, ?, 'selection', '{}', 'fixture-selection', ?, ?)",
        )
        .bind(CONVERSATION_ID)
        .bind(BOOK_ID)
        .bind(SECTION_ID)
        .bind("2026-01-01T00:00:00.000Z")
        .bind("2026-01-01T00:00:00.000Z")
        .execute(pool)
        .await
        .unwrap();
    }
    sqlx::query(
        "INSERT INTO annotations (
           id, book_id, section_id, kind, anchor_json, selected_text, note_text,
           conversation_id, created_at, updated_at
         ) VALUES (?, ?, ?, 'ai_conversation', '{}', 'fixture-selection', NULL, ?, ?, ?)",
    )
    .bind(ANNOTATION_ID)
    .bind(BOOK_ID)
    .bind(SECTION_ID)
    .bind(CONVERSATION_ID)
    .bind("2026-01-01T00:00:00.000Z")
    .bind("2026-01-01T00:00:00.000Z")
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO messages (
           id, conversation_id, ordinal, role, action, content, provider_id, model_id,
           citations_json, created_at
         ) VALUES (?, ?, 0, 'user', 'explain', 'fixture-question', NULL, NULL, NULL, ?)",
    )
    .bind(USER_MESSAGE_ID)
    .bind(CONVERSATION_ID)
    .bind("2026-01-01T00:00:00.000Z")
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO messages (
           id, conversation_id, ordinal, role, action, content, provider_id, model_id,
           citations_json, created_at
         ) VALUES (?, ?, 1, 'assistant', 'explain', 'fixture-answer', 'openai',
                   'fixture-model', '[]', ?)",
    )
    .bind(ASSISTANT_MESSAGE_ID)
    .bind(CONVERSATION_ID)
    .bind("2026-01-01T00:00:00.000Z")
    .execute(pool)
    .await
    .unwrap();

    if cutoff <= 4 {
        sqlx::query(
            "INSERT INTO provider_profiles (
               id, provider_kind, display_name, model_id, context_window_tokens, credential_key,
               is_active, created_at, updated_at
             ) VALUES (?, 'openai', 'fixture-profile', 'fixture-model', 32000, ?, 1, ?, ?)",
        )
        .bind(PROFILE_ID)
        .bind(format!("textbooklens/{PROFILE_ID}"))
        .bind("2026-01-01T00:00:00.000Z")
        .bind("2026-01-01T00:00:00.000Z")
        .execute(pool)
        .await
        .unwrap();
    } else if cutoff <= 6 {
        sqlx::query(
            "INSERT INTO provider_profiles (
               id, provider_kind, display_name, model_id, context_window_tokens, credential_key,
               is_active, created_at, updated_at, validated_at
             ) VALUES (?, 'openai', 'fixture-profile', 'fixture-model', 32000, ?, 1, ?, ?, ?)",
        )
        .bind(PROFILE_ID)
        .bind(format!("textbooklens/{PROFILE_ID}"))
        .bind("2026-01-01T00:00:00.000Z")
        .bind("2026-01-01T00:00:00.000Z")
        .bind("2026-01-01T00:00:00.000Z")
        .execute(pool)
        .await
        .unwrap();
    } else {
        sqlx::query(
            "INSERT INTO provider_profiles (
               id, provider_kind, display_name, model_id, context_window_tokens, is_active,
               created_at, updated_at, validated_at
             ) VALUES (?, 'openai', 'fixture-profile', 'fixture-model', 32000, 1, ?, ?, ?)",
        )
        .bind(PROFILE_ID)
        .bind("2026-01-01T00:00:00.000Z")
        .bind("2026-01-01T00:00:00.000Z")
        .bind("2026-01-01T00:00:00.000Z")
        .execute(pool)
        .await
        .unwrap();
    }
    if cutoff >= 7 {
        for category in ["image_send", "ai_index", "cost_risk"] {
            sqlx::query(
                "INSERT INTO provider_operation_consents (profile_id, category, decision, updated_at)
                 VALUES (?, ?, 'ask', ?)",
            )
            .bind(PROFILE_ID)
            .bind(category)
            .bind("2026-01-01T00:00:00.000Z")
            .execute(pool)
            .await
            .unwrap();
        }
        sqlx::query(
            "UPDATE app_settings
             SET active_provider_profile_id = ?, default_learning_profile_id = ?
             WHERE id = 1",
        )
        .bind(PROFILE_ID)
        .bind(PROFILE_ID)
        .execute(pool)
        .await
        .unwrap();
    } else {
        sqlx::query("UPDATE app_settings SET active_provider_profile_id = ? WHERE id = 1")
            .bind(PROFILE_ID)
            .execute(pool)
            .await
            .unwrap();
    }
}

async fn assert_current_contract(pool: &SqlitePool, expect_fixture: bool) {
    let foreign_keys: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(foreign_keys, 1);
    let integrity: Vec<String> = sqlx::query_scalar("PRAGMA integrity_check")
        .fetch_all(pool)
        .await
        .unwrap();
    assert_eq!(integrity, vec!["ok"]);
    assert!(
        sqlx::query("PRAGMA foreign_key_check")
            .fetch_optional(pool)
            .await
            .unwrap()
            .is_none()
    );

    let migrations = sqlx::query(
        "SELECT version, description, success, checksum FROM _sqlx_migrations ORDER BY version",
    )
    .fetch_all(pool)
    .await
    .unwrap();
    assert_eq!(migrations.len(), MIGRATOR.migrations.len());
    for (row, migration) in migrations.iter().zip(MIGRATOR.migrations.iter()) {
        assert_eq!(row.get::<i64, _>("version"), migration.version);
        assert_eq!(row.get::<String, _>("description"), migration.description);
        assert!(row.get::<bool, _>("success"));
        assert_eq!(
            row.get::<Vec<u8>, _>("checksum").as_slice(),
            migration.checksum.as_ref()
        );
    }

    let user_version: i64 = sqlx::query_scalar("PRAGMA user_version")
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(user_version, 0);
    if !expect_fixture {
        return;
    }

    let fixture_counts = [
        ("books", "SELECT COUNT(*) FROM books WHERE id = ?", BOOK_ID),
        (
            "sections",
            "SELECT COUNT(*) FROM sections WHERE id = ?",
            SECTION_ID,
        ),
        (
            "blocks",
            "SELECT COUNT(*) FROM blocks WHERE id = ?",
            BLOCK_ID,
        ),
        (
            "search_chunks",
            "SELECT COUNT(*) FROM search_chunks WHERE id = ?",
            CHUNK_ID,
        ),
        (
            "conversations",
            "SELECT COUNT(*) FROM conversations WHERE id = ?",
            CONVERSATION_ID,
        ),
        (
            "annotations",
            "SELECT COUNT(*) FROM annotations WHERE id = ?",
            ANNOTATION_ID,
        ),
        (
            "provider_profiles",
            "SELECT COUNT(*) FROM provider_profiles WHERE id = ?",
            PROFILE_ID,
        ),
    ];
    for (table, query, id) in fixture_counts {
        let count: i64 = sqlx::query_scalar(query)
            .bind(id)
            .fetch_one(pool)
            .await
            .unwrap();
        assert_eq!(count, 1, "fixture row lost from {table}");
    }
    let messages: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM messages WHERE conversation_id = ?")
            .bind(CONVERSATION_ID)
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(messages, 2);
    let anchor_kind: String =
        sqlx::query_scalar("SELECT anchor_kind FROM conversations WHERE id = ?")
            .bind(CONVERSATION_ID)
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(anchor_kind, "text");
    let credential_columns: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pragma_table_info('provider_profiles') WHERE name = 'credential_key'",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(credential_columns, 0);
    let fts_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM search_chunks_fts WHERE search_chunks_fts MATCH 'migrationfixtureterm'",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(fts_count, 1);

    assert!(
        sqlx::query(
            "INSERT INTO books (
               id, title, format, original_filename, import_status, created_at, updated_at
             ) VALUES (?, 'invalid', 'binary', 'invalid.bin', 'queued', ?, ?)",
        )
        .bind(Uuid::new_v4().to_string())
        .bind("2026-01-01T00:00:00.000Z")
        .bind("2026-01-01T00:00:00.000Z")
        .execute(pool)
        .await
        .is_err()
    );
    assert!(
        sqlx::query("UPDATE app_settings SET ui_language = 'invalid' WHERE id = 1")
            .execute(pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("DELETE FROM teaching_preferences WHERE id = 1")
            .execute(pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("DELETE FROM panel_preferences WHERE id = 1")
            .execute(pool)
            .await
            .is_err()
    );
}

async fn schema_rows(pool: &SqlitePool) -> Vec<SchemaRow> {
    sqlx::query_as(
        "SELECT type, name, tbl_name, sql
         FROM sqlite_schema
         WHERE name NOT GLOB 'sqlite_*'
         ORDER BY type, name, tbl_name",
    )
    .fetch_all(pool)
    .await
    .unwrap()
}

async fn object_exists(pool: &SqlitePool, name: &str) -> bool {
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sqlite_schema WHERE name = ?")
        .bind(name)
        .fetch_one(pool)
        .await
        .unwrap()
        != 0
}

async fn create_migration_history_table(pool: &SqlitePool) {
    sqlx::raw_sql(
        "CREATE TABLE _sqlx_migrations (
           version BIGINT PRIMARY KEY,
           description TEXT NOT NULL,
           installed_on TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
           success BOOLEAN NOT NULL,
           checksum BLOB NOT NULL,
           execution_time BIGINT NOT NULL
         );",
    )
    .execute(pool)
    .await
    .unwrap();
}

async fn insert_migration_row(
    pool: &SqlitePool,
    migration: &sqlx::migrate::Migration,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO _sqlx_migrations (
           version, description, success, checksum, execution_time
         ) VALUES (?, ?, 1, ?, 0)",
    )
    .bind(migration.version)
    .bind(migration.description.as_ref())
    .bind(migration.checksum.as_ref())
    .execute(pool)
    .await?;
    Ok(())
}

fn hex(bytes: impl AsRef<[u8]>) -> String {
    bytes
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn block_on<F: std::future::Future>(future: F) -> F::Output {
    tauri::async_runtime::block_on(future)
}

#[cfg(windows)]
fn create_windows_junction(source: &Path, target: &Path) {
    let status = Command::new("cmd")
        .args([
            "/c",
            "mklink",
            "/J",
            &target.to_string_lossy(),
            &source.to_string_lossy(),
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success());
}

#[cfg(windows)]
fn set_windows_readonly(path: &Path, readonly: bool) {
    use std::os::windows::ffi::OsStrExt;

    const FILE_ATTRIBUTE_READONLY: u32 = 0x0000_0001;
    const INVALID_FILE_ATTRIBUTES: u32 = u32::MAX;
    let mut wide = path.as_os_str().encode_wide().collect::<Vec<_>>();
    wide.push(0);
    // SAFETY: `wide` is a live NUL-terminated UTF-16 path buffer.
    let attributes = unsafe { get_file_attributes_w_for_test(wide.as_ptr()) };
    assert_ne!(attributes, INVALID_FILE_ATTRIBUTES);
    let updated = if readonly {
        attributes | FILE_ATTRIBUTE_READONLY
    } else {
        attributes & !FILE_ATTRIBUTE_READONLY
    };
    // SAFETY: `wide` remains live and `updated` preserves every unrelated bit.
    assert_ne!(
        unsafe { set_file_attributes_w_for_test(wide.as_ptr(), updated) },
        0
    );
}

#[cfg(windows)]
struct ExclusiveWindowsFileLock(*mut std::ffi::c_void);

#[cfg(windows)]
impl ExclusiveWindowsFileLock {
    fn acquire(path: &Path) -> Self {
        use std::os::windows::ffi::OsStrExt;

        const GENERIC_READ: u32 = 0x8000_0000;
        const OPEN_EXISTING: u32 = 3;
        const FILE_ATTRIBUTE_NORMAL: u32 = 0x0000_0080;
        let mut wide = path.as_os_str().encode_wide().collect::<Vec<_>>();
        wide.push(0);
        // SAFETY: `wide` is a live NUL-terminated UTF-16 path buffer. The
        // returned handle is checked and owned by this guard.
        let handle = unsafe {
            create_file_w_for_test(
                wide.as_ptr(),
                GENERIC_READ,
                0,
                std::ptr::null_mut(),
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                std::ptr::null_mut(),
            )
        };
        assert_ne!(handle as isize, -1);
        Self(handle)
    }
}

#[cfg(windows)]
impl Drop for ExclusiveWindowsFileLock {
    fn drop(&mut self) {
        // SAFETY: the handle is valid and closed exactly once by this guard.
        let _ = unsafe { close_handle_for_test(self.0) };
    }
}

#[cfg(windows)]
unsafe extern "system" {
    #[link_name = "GetFileAttributesW"]
    fn get_file_attributes_w_for_test(file_name: *const u16) -> u32;

    #[link_name = "SetFileAttributesW"]
    fn set_file_attributes_w_for_test(file_name: *const u16, file_attributes: u32) -> i32;

    #[link_name = "CreateFileW"]
    fn create_file_w_for_test(
        file_name: *const u16,
        desired_access: u32,
        share_mode: u32,
        security_attributes: *mut std::ffi::c_void,
        creation_disposition: u32,
        flags_and_attributes: u32,
        template_file: *mut std::ffi::c_void,
    ) -> *mut std::ffi::c_void;

    #[link_name = "CloseHandle"]
    fn close_handle_for_test(handle: *mut std::ffi::c_void) -> i32;
}
