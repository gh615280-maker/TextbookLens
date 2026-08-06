use std::fs;

use secrecy::SecretString;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use sqlx::Row;
use tempfile::TempDir;
use textbooklens_lib::{
    app_state::AppPaths,
    credentials::{CredentialStore, MemoryCredentialStore},
    db::{Database, providers::credential_key},
    domain::{ClearAllDataStatusCode, RestoreBackupStatusCode},
    maintenance::{
        archive::{BACKUP_FORMAT_VERSION, BackupService},
        clear_all::{
            CLEAR_ALL_DATA_CONFIRMATION_PHRASE, ClearAllDataService, ClearStartupOutcome,
            recover_pending_clear,
        },
        gate::MaintenanceGate,
        restore::{RestoreService, recover_pending_restore},
        storage::{APP_DATA_DIRECTORY_NAME, prepare_app_data_paths},
    },
};
use uuid::Uuid;

const TIMESTAMP: &str = "2026-08-06T00:00:00.000Z";
const SYNTHETIC_KEY_SENTINEL: &str = concat!("sk", "-", "synthetic-checkpoint-key-never-archive");
const PRIVATE_BODY_SENTINEL: &str = "PRIVATE_BODY_MUST_NOT_LEAK";
const PRIVATE_IMAGE_SENTINEL: &str = "PRIVATE_IMAGE_PAYLOAD_MUST_NOT_LEAK";
const PRIVATE_PROMPT_SENTINEL: &str = "PRIVATE_PROMPT_MUST_NOT_LEAK";
const PRIVATE_ANSWER_SENTINEL: &str = "PRIVATE_ANSWER_MUST_NOT_LEAK";
const REMOTE_REFERENCE_SENTINEL: &str = "enc:v1:keyring:10000000-0000-4000-8000-000000000099";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Manifest {
    format: String,
    version: u32,
    entry_count: usize,
    total_bytes: u64,
    entries: Vec<ManifestEntry>,
}

#[derive(Debug, Deserialize)]
struct ManifestEntry {
    path: String,
    offset: u64,
    size: u64,
    sha256: String,
}

struct BookSeed {
    id: Uuid,
    format: &'static str,
    bytes: &'static [u8],
    user_source: std::path::PathBuf,
}

#[test]
fn scenario_p_real_v1_backup_restores_three_formats_without_local_credentials() {
    let source_temp = TempDir::new().unwrap();
    let source_paths = app_paths(&source_temp);
    let source_database = Database::open(&source_paths.database).unwrap();
    tauri::async_runtime::block_on(async {
        let profile_id = uuid::uuid!("10000000-0000-4000-8000-000000000001");
        seed_profile(source_database.pool(), profile_id).await;

        let credential_store = MemoryCredentialStore::new();
        let credential_identifier = credential_key(profile_id);
        credential_store
            .set(
                &credential_identifier,
                SecretString::from(SYNTHETIC_KEY_SENTINEL),
            )
            .await
            .unwrap();

        let books = vec![
            seed_book(
                source_database.pool(),
                &source_paths,
                &source_temp,
                uuid::uuid!("10000000-0000-4000-8000-000000000010"),
                "pdf",
                b"%PDF-1.7\nSYNTHETIC_DURABLE_PDF_COPY\n%%EOF",
            )
            .await,
            seed_book(
                source_database.pool(),
                &source_paths,
                &source_temp,
                uuid::uuid!("10000000-0000-4000-8000-000000000011"),
                "epub",
                b"PK\x03\x04SYNTHETIC_DURABLE_EPUB_COPY",
            )
            .await,
            seed_book(
                source_database.pool(),
                &source_paths,
                &source_temp,
                uuid::uuid!("10000000-0000-4000-8000-000000000012"),
                "docx",
                b"PK\x03\x04SYNTHETIC_DURABLE_DOCX_COPY",
            )
            .await,
        ];
        seed_durable_state(source_database.pool(), books[0].id, profile_id).await;
        seed_forbidden_runtime_artifacts(&source_paths);

        let remote_id = uuid::uuid!("10000000-0000-4000-8000-000000000099");
        sqlx::query(
        "INSERT INTO provider_remote_resources (id, book_id, provider_profile_id, provider_kind, encrypted_reference, cleanup_status, created_at, updated_at) VALUES (?, ?, ?, 'openai', ?, 'pending', ?, ?)",
    )
    .bind(remote_id.to_string())
    .bind(books[0].id.to_string())
    .bind(profile_id.to_string())
    .bind(REMOTE_REFERENCE_SENTINEL)
    .bind(TIMESTAMP)
    .bind(TIMESTAMP)
    .execute(source_database.pool())
    .await
    .unwrap();

        let archive = source_temp.path().join("scenario-p.tlbackup");
        let summary = BackupService::new(
            source_database.pool().clone(),
            source_paths.clone(),
            MaintenanceGate::default(),
        )
        .create_backup(archive.clone())
        .await
        .unwrap();
        assert_eq!(summary.format_version, BACKUP_FORMAT_VERSION);
        assert_eq!(summary.entry_count, 7);

        let archive_bytes = fs::read(&archive).unwrap();
        let manifest = read_manifest(&archive_bytes);
        assert_eq!(manifest.format, "textbooklens-local-backup");
        assert_eq!(manifest.version, BACKUP_FORMAT_VERSION);
        assert_eq!(manifest.entry_count, 7);
        assert_eq!(manifest.entries.len(), 7);
        assert_eq!(manifest.entries[0].path, "library.sqlite3");
        assert_eq!(
            manifest.total_bytes,
            manifest.entries.iter().map(|entry| entry.size).sum::<u64>()
        );
        assert_manifest_entries(&archive_bytes, &manifest);
        for book in &books {
            assert!(manifest.entries.iter().any(|entry| {
                entry.path == format!("books/{}/original.{}", book.id, book.format)
            }));
            assert!(contains_bytes(&archive_bytes, book.bytes));
        }

        let source_absolute = books[0].user_source.to_string_lossy().into_owned();
        for forbidden in [
            SYNTHETIC_KEY_SENTINEL,
            credential_identifier.as_str(),
            REMOTE_REFERENCE_SENTINEL,
            PRIVATE_BODY_SENTINEL,
            PRIVATE_IMAGE_SENTINEL,
            PRIVATE_PROMPT_SENTINEL,
            PRIVATE_ANSWER_SENTINEL,
            source_absolute.as_str(),
        ] {
            assert!(
                !contains_bytes(&archive_bytes, forbidden.as_bytes()),
                "archive leaked isolated sentinel"
            );
        }

        let restore_temp = TempDir::new().unwrap();
        let restore_paths = app_paths(&restore_temp);
        let current_database = {
            let database_path = restore_paths.database.clone();
            tokio::task::spawn_blocking(move || Database::open(database_path))
                .await
                .unwrap()
                .unwrap()
        };
        sqlx::query("UPDATE app_settings SET ui_language = 'en' WHERE id = 1")
            .execute(current_database.pool())
            .await
            .unwrap();
        let no_local_credentials = std::sync::Arc::new(MemoryCredentialStore::new());
        let restore = RestoreService::new(
            restore_paths.clone(),
            MaintenanceGate::default(),
            no_local_credentials,
        )
        .stage_restore(archive.clone())
        .await
        .unwrap();
        assert_eq!(restore.status, RestoreBackupStatusCode::ReadyToRestart);
        assert!(restore.restart_required);
        assert!(restore.ai_configuration_required);
        current_database.pool().close().await;

        assert!(recover_pending_restore(&restore_paths.root).unwrap());
        assert!(!recover_pending_restore(&restore_paths.root).unwrap());
        let restored = {
            let database_path = restore_paths.database.clone();
            tokio::task::spawn_blocking(move || Database::open(database_path))
                .await
                .unwrap()
                .unwrap()
        };
        assert_restored_state(restored.pool(), &restore_paths, &books, profile_id).await;
        restored.pool().close().await;

        assert!(archive.exists());
        for book in &books {
            assert_eq!(fs::read(&book.user_source).unwrap(), book.bytes);
        }
    });
}

#[test]
fn scenario_q_clear_retry_and_restart_remove_owned_data_but_preserve_external_decoys() {
    let temporary = TempDir::new().unwrap();
    let paths = app_paths(&temporary);
    let database = Database::open(&paths.database).unwrap();
    tauri::async_runtime::block_on(async {
        let profile_id = uuid::uuid!("20000000-0000-4000-8000-000000000001");
        let remote_id = uuid::uuid!("20000000-0000-4000-8000-000000000002");
        seed_profile(database.pool(), profile_id).await;
        sqlx::query(
        "INSERT INTO provider_remote_resources (id, provider_kind, encrypted_reference, cleanup_status, created_at, updated_at) VALUES (?, 'openai', ?, 'pending', ?, ?)",
    )
    .bind(remote_id.to_string())
    .bind(format!("enc:v1:keyring:{remote_id}"))
    .bind(TIMESTAMP)
    .bind(TIMESTAMP)
    .execute(database.pool())
    .await
    .unwrap();
        fs::write(paths.books.join("owned.bin"), b"OWNED_CLEAR_SENTINEL").unwrap();
        fs::write(paths.cache.join("index.bin"), b"OWNED_INDEX_SENTINEL").unwrap();
        fs::write(paths.logs.join("diagnostic.log"), b"OWNED_LOG_SENTINEL").unwrap();

        let original = temporary.path().join("user-original.pdf");
        let external_backup = temporary.path().join("external.tlbackup");
        let sibling_decoy = temporary.path().join("decoy-sibling.bin");
        fs::write(&original, b"USER_ORIGINAL_PRESERVED").unwrap();
        fs::write(&external_backup, b"EXTERNAL_BACKUP_PRESERVED").unwrap();
        fs::write(&sibling_decoy, b"SIBLING_DECOY_PRESERVED").unwrap();

        let credentials = std::sync::Arc::new(MemoryCredentialStore::new());
        credentials
            .set(
                &credential_key(profile_id),
                SecretString::from(SYNTHETIC_KEY_SENTINEL),
            )
            .await
            .unwrap();
        credentials
            .set(
                &format!("textbooklens/remote-resource/{remote_id}"),
                SecretString::from("synthetic-remote-secret"),
            )
            .await
            .unwrap();
        credentials.fail_next_delete();
        let service = ClearAllDataService::new(
            database.pool().clone(),
            paths.clone(),
            MaintenanceGate::default(),
            credentials.clone(),
        );

        let partial = service
            .clear_all_data(CLEAR_ALL_DATA_CONFIRMATION_PHRASE.to_owned())
            .await
            .unwrap();
        assert_eq!(
            partial.status,
            ClearAllDataStatusCode::CredentialCleanupRequired
        );
        assert!(!partial.restart_required);
        assert!(paths.database.exists());

        let ready = service
            .clear_all_data(CLEAR_ALL_DATA_CONFIRMATION_PHRASE.to_owned())
            .await
            .unwrap();
        assert_eq!(ready.status, ClearAllDataStatusCode::ReadyToRestart);
        assert!(ready.restart_required);
        database.pool().close().await;

        assert_eq!(
            recover_pending_clear(&paths.root, credentials.as_ref())
                .await
                .unwrap(),
            ClearStartupOutcome::Cleared
        );
        assert_eq!(
            recover_pending_clear(&paths.root, credentials.as_ref())
                .await
                .unwrap(),
            ClearStartupOutcome::NoPending
        );
        assert!(
            credentials
                .list_textbooklens_keys()
                .await
                .unwrap()
                .is_empty()
        );
        for owned in [&paths.database, &paths.books, &paths.cache, &paths.logs] {
            assert!(!owned.exists());
        }
        assert_eq!(fs::read(&original).unwrap(), b"USER_ORIGINAL_PRESERVED");
        assert_eq!(
            fs::read(&external_backup).unwrap(),
            b"EXTERNAL_BACKUP_PRESERVED"
        );
        assert_eq!(
            fs::read(&sibling_decoy).unwrap(),
            b"SIBLING_DECOY_PRESERVED"
        );
    });
}

fn app_paths(temporary: &TempDir) -> AppPaths {
    let prepared = prepare_app_data_paths(&temporary.path().join(APP_DATA_DIRECTORY_NAME)).unwrap();
    AppPaths {
        root: prepared.root,
        books: prepared.books,
        cache: prepared.cache,
        logs: prepared.logs,
        database: prepared.database,
    }
}

async fn seed_profile(pool: &sqlx::SqlitePool, profile_id: Uuid) {
    sqlx::query(
        "INSERT INTO provider_profiles (id, provider_kind, display_name, model_id, context_window_tokens, is_active, created_at, updated_at, validated_at) VALUES (?, 'openai', 'Synthetic checkpoint profile', 'synthetic-model', 100000, 1, ?, ?, ?)",
    )
    .bind(profile_id.to_string())
    .bind(TIMESTAMP)
    .bind(TIMESTAMP)
    .bind(TIMESTAMP)
    .execute(pool)
    .await
    .unwrap();
}

async fn seed_book(
    pool: &sqlx::SqlitePool,
    paths: &AppPaths,
    temporary: &TempDir,
    id: Uuid,
    format: &'static str,
    bytes: &'static [u8],
) -> BookSeed {
    let book = paths.books.join(id.to_string());
    fs::create_dir_all(book.join("derived")).unwrap();
    let owned = book.join(format!("original.{format}"));
    fs::write(&owned, bytes).unwrap();
    fs::write(
        book.join("derived/document.html"),
        format!("<article>SYNTHETIC_DURABLE_{format}_DERIVED</article>"),
    )
    .unwrap();
    let user_source = temporary
        .path()
        .join(format!("user-source-{format}.{format}"));
    fs::write(&user_source, bytes).unwrap();
    sqlx::query(
        "INSERT INTO books (id, sha256, title, author, language, format, original_filename, stored_path, import_status, created_at, updated_at, reading_progress, last_locator_json) VALUES (?, ?, ?, 'Synthetic author', 'en', ?, ?, ?, 'ready', ?, ?, 0.5, ?)",
    )
    .bind(id.to_string())
    .bind(hex(Sha256::digest(bytes)))
    .bind(format!("Synthetic {format} checkpoint"))
    .bind(format)
    .bind(format!("checkpoint.{format}"))
    .bind(format!("books/{id}/original.{format}"))
    .bind(TIMESTAMP)
    .bind(TIMESTAMP)
    .bind(format!(r#"{{"format":"{format}","position":1}}"#))
    .execute(pool)
    .await
    .unwrap();
    BookSeed {
        id,
        format,
        bytes,
        user_source,
    }
}

async fn seed_durable_state(pool: &sqlx::SqlitePool, book_id: Uuid, profile_id: Uuid) {
    let section_id = uuid::uuid!("10000000-0000-4000-8000-000000000020");
    let block_id = uuid::uuid!("10000000-0000-4000-8000-000000000021");
    let conversation_id = uuid::uuid!("10000000-0000-4000-8000-000000000022");
    let run_id = uuid::uuid!("10000000-0000-4000-8000-000000000023");
    let page_id = uuid::uuid!("10000000-0000-4000-8000-000000000024");
    let attempt_id = uuid::uuid!("10000000-0000-4000-8000-000000000025");
    let index_block_id = uuid::uuid!("10000000-0000-4000-8000-000000000026");
    let correction_id = uuid::uuid!("10000000-0000-4000-8000-000000000027");
    let source_hash: String = sqlx::query_scalar("SELECT sha256 FROM books WHERE id = ?")
        .bind(book_id.to_string())
        .fetch_one(pool)
        .await
        .unwrap();
    sqlx::query("UPDATE app_settings SET onboarding_completed = 1, active_provider_profile_id = ?, default_learning_profile_id = ?, theme = 'dark', context_mode = 'long', ui_language = 'zh-TW', ui_language_initialized = 1, first_reader_hint_completed = 1, font_scale = 1.25, line_height = 1.8, reader_width = 80, pdf_zoom = 1.5 WHERE id = 1")
        .bind(profile_id.to_string())
        .bind(profile_id.to_string())
        .execute(pool).await.unwrap();
    sqlx::query("UPDATE teaching_preferences SET instruction = 'Use durable synthetic checkpoints.', revision = 3, updated_at = ? WHERE id = 1")
        .bind(TIMESTAMP).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO sections (id, book_id, ordinal, title, locator_json) VALUES (?, ?, 0, 'Synthetic section', '{}')")
        .bind(section_id.to_string()).bind(book_id.to_string()).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO blocks (id, book_id, section_id, ordinal, kind, plain_text, locator_json) VALUES (?, ?, ?, 0, 'paragraph', 'Durable synthetic local text', '{}')")
        .bind(block_id.to_string()).bind(book_id.to_string()).bind(section_id.to_string()).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO search_chunks (id, book_id, section_id, ordinal, text, locator_json, token_estimate) VALUES (?, ?, ?, 0, 'Durable synthetic searchable text', '{}', 4)")
        .bind(Uuid::new_v4().to_string()).bind(book_id.to_string()).bind(section_id.to_string()).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO conversations (id, book_id, section_id, scope, anchor_kind, anchor_json, selected_text, created_at, updated_at) VALUES (?, ?, ?, 'selection', 'text', '{}', 'Durable selection', ?, ?)")
        .bind(conversation_id.to_string()).bind(book_id.to_string()).bind(section_id.to_string()).bind(TIMESTAMP).bind(TIMESTAMP).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO messages (id, conversation_id, ordinal, role, action, content, created_at) VALUES (?, ?, 0, 'user', 'explain', 'Durable synthetic question', ?)")
        .bind(Uuid::new_v4().to_string()).bind(conversation_id.to_string()).bind(TIMESTAMP).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO messages (id, conversation_id, ordinal, role, action, content, provider_id, model_id, citations_json, created_at) VALUES (?, ?, 1, 'assistant', 'explain', 'Durable synthetic completion', ?, 'synthetic-model', '[]', ?)")
        .bind(Uuid::new_v4().to_string()).bind(conversation_id.to_string()).bind(profile_id.to_string()).bind(TIMESTAMP).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO annotations (id, book_id, section_id, kind, anchor_json, selected_text, conversation_id, revision, created_at, updated_at) VALUES (?, ?, ?, 'ai_conversation', '{}', 'Durable selection', ?, 1, ?, ?)")
        .bind(Uuid::new_v4().to_string()).bind(book_id.to_string()).bind(section_id.to_string()).bind(conversation_id.to_string()).bind(TIMESTAMP).bind(TIMESTAMP).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO annotations (id, book_id, section_id, kind, anchor_json, selected_text, note_text, revision, created_at, updated_at) VALUES (?, ?, ?, 'note', '{}', 'Durable selection', 'Durable synthetic note', 1, ?, ?)")
        .bind(Uuid::new_v4().to_string()).bind(book_id.to_string()).bind(section_id.to_string()).bind(TIMESTAMP).bind(TIMESTAMP).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO index_runs (id, book_id, source_sha256, provider_profile_id, provider_kind, model_id, analysis_schema_version, render_version, parser_version, status, created_at, updated_at, completed_at) VALUES (?, ?, ?, ?, 'openai', 'synthetic-model', 'v1', 'v1', 'v1', 'completed', ?, ?, ?)")
        .bind(run_id.to_string()).bind(book_id.to_string()).bind(source_hash).bind(profile_id.to_string()).bind(TIMESTAMP).bind(TIMESTAMP).bind(TIMESTAMP).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO index_pages (id, run_id, book_id, page_number, quality_reason, status, attempt_id, attempt_count, content_sha256, content_version, created_at, updated_at) VALUES (?, ?, ?, 1, 'no_text', 'indexed', ?, 1, ?, 1, ?, ?)")
        .bind(page_id.to_string()).bind(run_id.to_string()).bind(book_id.to_string()).bind(attempt_id.to_string()).bind("c".repeat(64)).bind(TIMESTAMP).bind(TIMESTAMP).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO index_page_blocks (id, page_id, run_id, book_id, ordinal, kind, plain_text, source, provenance_version, content_version, value_sha256, created_at) VALUES (?, ?, ?, ?, 0, 'paragraph', 'Durable indexed source', 'ai_transcribed', 1, 1, ?, ?)")
        .bind(index_block_id.to_string()).bind(page_id.to_string()).bind(run_id.to_string()).bind(book_id.to_string()).bind("d".repeat(64)).bind(TIMESTAMP).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO index_corrections (id, book_id, page_id, target_block_id, target_content_version, value_kind, original_value_sha256, original_value, corrected_value, conflict_state, revision, created_at, updated_at) VALUES (?, ?, ?, ?, 1, 'text', ?, 'Durable indexed source', 'Durable corrected source', 'active', 1, ?, ?)")
        .bind(correction_id.to_string()).bind(book_id.to_string()).bind(page_id.to_string()).bind(index_block_id.to_string()).bind("d".repeat(64)).bind(TIMESTAMP).bind(TIMESTAMP).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO index_search_chunks (id, book_id, page_id, block_id, correction_id, ordinal, source, text, locator_json, token_estimate, content_version, created_at) VALUES (?, ?, ?, ?, ?, 0, 'user_corrected', 'Durable corrected searchable source', '{}', 4, 1, ?)")
        .bind(Uuid::new_v4().to_string()).bind(book_id.to_string()).bind(page_id.to_string()).bind(index_block_id.to_string()).bind(correction_id.to_string()).bind(TIMESTAMP).execute(pool).await.unwrap();
}

fn seed_forbidden_runtime_artifacts(paths: &AppPaths) {
    fs::write(
        paths.logs.join("textbooklens.log"),
        format!("{PRIVATE_BODY_SENTINEL}\n{PRIVATE_PROMPT_SENTINEL}\n{PRIVATE_ANSWER_SENTINEL}"),
    )
    .unwrap();
    fs::write(
        paths.cache.join("page-image.partial"),
        PRIVATE_IMAGE_SENTINEL,
    )
    .unwrap();
}

fn read_manifest(archive: &[u8]) -> Manifest {
    assert_eq!(&archive[..8], b"TLBACKUP");
    assert_eq!(
        u32::from_le_bytes(archive[8..12].try_into().unwrap()),
        BACKUP_FORMAT_VERSION
    );
    let footer = archive.len() - 48;
    assert_eq!(&archive[footer..footer + 8], b"TLBEND01");
    let manifest_len = usize::try_from(u64::from_le_bytes(
        archive[footer + 8..footer + 16].try_into().unwrap(),
    ))
    .unwrap();
    serde_json::from_slice(&archive[footer - manifest_len..footer]).unwrap()
}

fn assert_manifest_entries(archive: &[u8], manifest: &Manifest) {
    let mut expected_offset = 16_u64;
    let mut paths = std::collections::HashSet::new();
    for entry in &manifest.entries {
        assert_eq!(entry.offset, expected_offset);
        assert!(paths.insert(entry.path.clone()));
        assert!(!entry.path.starts_with('/') && !entry.path.contains(['\\', ':']));
        assert!(!entry.path.split('/').any(|component| component == ".."));
        let start = usize::try_from(entry.offset).unwrap();
        let end = usize::try_from(entry.offset + entry.size).unwrap();
        assert_eq!(hex(Sha256::digest(&archive[start..end])), entry.sha256);
        expected_offset += entry.size;
    }
}

async fn assert_restored_state(
    pool: &sqlx::SqlitePool,
    paths: &AppPaths,
    books: &[BookSeed],
    profile_id: Uuid,
) {
    let rows = sqlx::query("SELECT id, format, sha256, stored_path FROM books ORDER BY id")
        .fetch_all(pool)
        .await
        .unwrap();
    assert_eq!(rows.len(), 3);
    for book in books {
        let row = rows
            .iter()
            .find(|row| row.get::<String, _>("id") == book.id.to_string())
            .unwrap();
        let stored_path = format!("books/{}/original.{}", book.id, book.format);
        assert_eq!(row.get::<String, _>("format"), book.format);
        assert_eq!(row.get::<String, _>("stored_path"), stored_path);
        assert_eq!(
            row.get::<String, _>("sha256"),
            hex(Sha256::digest(book.bytes))
        );
        let restored_file = paths
            .books
            .join(book.id.to_string())
            .join(format!("original.{}", book.format));
        assert!(restored_file.starts_with(&paths.books));
        assert_eq!(fs::read(restored_file).unwrap(), book.bytes);
        assert!(
            paths
                .books
                .join(book.id.to_string())
                .join("derived/document.html")
                .is_file()
        );
    }
    for (table, query, expected) in [
        ("index_runs", "SELECT COUNT(*) FROM index_runs", 1_i64),
        ("index_pages", "SELECT COUNT(*) FROM index_pages", 1),
        (
            "index_corrections",
            "SELECT COUNT(*) FROM index_corrections",
            1,
        ),
        (
            "index_search_chunks",
            "SELECT COUNT(*) FROM index_search_chunks",
            1,
        ),
        ("conversations", "SELECT COUNT(*) FROM conversations", 1),
        ("messages", "SELECT COUNT(*) FROM messages", 2),
        ("annotations", "SELECT COUNT(*) FROM annotations", 2),
        (
            "provider_remote_resources",
            "SELECT COUNT(*) FROM provider_remote_resources",
            0,
        ),
    ] {
        let count: i64 = sqlx::query_scalar(query).fetch_one(pool).await.unwrap();
        assert_eq!(count, expected, "wrong restored count for {table}");
    }
    let settings = sqlx::query("SELECT ui_language, theme, context_mode, default_learning_profile_id, font_scale, line_height, reader_width, pdf_zoom FROM app_settings WHERE id = 1")
        .fetch_one(pool).await.unwrap();
    assert_eq!(settings.get::<String, _>("ui_language"), "zh-TW");
    assert_eq!(settings.get::<String, _>("theme"), "dark");
    assert_eq!(settings.get::<String, _>("context_mode"), "long");
    assert_eq!(
        settings.get::<String, _>("default_learning_profile_id"),
        profile_id.to_string()
    );
    assert_eq!(settings.get::<f64, _>("font_scale"), 1.25);
    assert_eq!(settings.get::<f64, _>("line_height"), 1.8);
    assert_eq!(settings.get::<f64, _>("reader_width"), 80.0);
    assert_eq!(settings.get::<f64, _>("pdf_zoom"), 1.5);
    let instruction: String =
        sqlx::query_scalar("SELECT instruction FROM teaching_preferences WHERE id = 1")
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(instruction, "Use durable synthetic checkpoints.");
    let integrity: String = sqlx::query_scalar("PRAGMA integrity_check")
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(integrity, "ok");
    assert!(
        sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(pool)
            .await
            .unwrap()
            .is_empty()
    );
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && haystack
            .windows(needle.len())
            .any(|candidate| candidate == needle)
}

fn hex(bytes: impl AsRef<[u8]>) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.as_ref().len() * 2);
    for byte in bytes.as_ref() {
        output.push(char::from(DIGITS[usize::from(byte >> 4)]));
        output.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    output
}
