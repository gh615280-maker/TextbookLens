use std::{fs, time::Duration};

use chrono::{DateTime, Utc};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::{
    db::{
        Database,
        indexing::{CreateIndexRun, IndexPageOwnership, create_index_run},
    },
    domain::{IndexFailureCode, IndexPageStatus, IndexQualityReason},
    errors::AppErrorCode,
};

use super::{
    recovery::{recover_interrupted_pages, scratch_path},
    state::{
        claim_render, claim_send, claim_validate, commit_indexed, mark_received, mark_rendered,
        queue,
    },
};

#[test]
fn startup_recovery_requeues_fresh_or_rendering_and_marks_missing_or_stale_retryable() {
    let temporary = tempfile::tempdir().unwrap();
    let database = Database::open(temporary.path().join("recovery.sqlite3")).unwrap();
    let scratch_root = temporary.path().join("indexing-scratch");

    tauri::async_runtime::block_on(async {
        let fixture = create_recovery_fixture(database.pool(), 5).await;
        let rendering = fixture.pages[0];
        let sending = fixture.pages[1];
        let parsing = fixture.pages[2];
        let validating = fixture.pages[3];
        let indexed = fixture.pages[4];

        claim_render(
            database.pool(),
            rendering.page_id,
            rendering.attempt_id.unwrap(),
        )
        .await
        .unwrap();
        drive_to_sending(database.pool(), sending).await;
        drive_to_parsing(database.pool(), parsing).await;
        drive_to_validating(database.pool(), validating).await;
        drive_to_validating(database.pool(), indexed).await;
        commit_indexed(
            database.pool(),
            indexed.page_id,
            indexed.attempt_id.unwrap(),
            &"c".repeat(64),
        )
        .await
        .unwrap();

        set_updated_at(
            database.pool(),
            rendering.page_id,
            "2026-08-04T00:55:00.000Z",
        )
        .await;
        set_updated_at(database.pool(), sending.page_id, "2026-08-04T00:55:00.000Z").await;
        set_updated_at(database.pool(), parsing.page_id, "2026-08-04T00:55:00.000Z").await;
        set_updated_at(
            database.pool(),
            validating.page_id,
            "2026-08-04T00:00:00.000Z",
        )
        .await;
        set_updated_at(database.pool(), indexed.page_id, "2026-08-04T00:30:00.000Z").await;

        let sending_scratch =
            scratch_path(&scratch_root, sending.page_id, sending.attempt_id.unwrap());
        fs::create_dir_all(sending_scratch.parent().unwrap()).unwrap();
        fs::write(&sending_scratch, b"synthetic-render").unwrap();
        let validating_scratch = scratch_path(
            &scratch_root,
            validating.page_id,
            validating.attempt_id.unwrap(),
        );
        fs::create_dir_all(validating_scratch.parent().unwrap()).unwrap();
        fs::write(&validating_scratch, b"stale-synthetic-render").unwrap();

        let indexed_before = page_recovery_row(database.pool(), indexed.page_id).await;
        let now = DateTime::parse_from_rfc3339("2026-08-04T01:00:00.000Z")
            .unwrap()
            .with_timezone(&Utc);
        let summary = recover_interrupted_pages(
            database.pool(),
            &scratch_root,
            now,
            Duration::from_secs(10 * 60),
        )
        .await
        .unwrap();
        assert_eq!(summary.requeued, 2);
        assert_eq!(summary.marked_retryable, 2);
        assert_eq!(summary.unchanged_indexed, 1);

        let rendering_after = page_recovery_row(database.pool(), rendering.page_id).await;
        assert_eq!(rendering_after.status, IndexPageStatus::Queued);
        assert_ne!(rendering_after.attempt_id, rendering.attempt_id.unwrap());
        assert_eq!(rendering_after.attempt_count, 2);
        assert!(!rendering_after.has_render_hash);

        let sending_after = page_recovery_row(database.pool(), sending.page_id).await;
        assert_eq!(sending_after.status, IndexPageStatus::Queued);
        assert_ne!(sending_after.attempt_id, sending.attempt_id.unwrap());
        assert_eq!(sending_after.attempt_count, 2);
        assert!(sending_after.has_render_hash);
        assert!(!sending_scratch.exists());
        assert!(scratch_path(&scratch_root, sending.page_id, sending_after.attempt_id,).exists());

        let parsing_after = page_recovery_row(database.pool(), parsing.page_id).await;
        assert_eq!(parsing_after.status, IndexPageStatus::Failed);
        assert_eq!(parsing_after.attempt_id, parsing.attempt_id.unwrap());
        assert_eq!(
            parsing_after.safe_error_code.as_deref(),
            Some(IndexFailureCode::IndexScratchMissing.as_str())
        );
        assert!(parsing_after.retryable);

        let validating_after = page_recovery_row(database.pool(), validating.page_id).await;
        assert_eq!(validating_after.status, IndexPageStatus::Failed);
        assert_eq!(validating_after.attempt_id, validating.attempt_id.unwrap());
        assert_eq!(
            validating_after.safe_error_code.as_deref(),
            Some(IndexFailureCode::IndexAttemptExpired.as_str())
        );
        assert!(validating_after.retryable);
        assert!(!validating_scratch.exists());

        let indexed_after = page_recovery_row(database.pool(), indexed.page_id).await;
        assert_eq!(indexed_after, indexed_before);

        let late_old_attempt = mark_received(
            database.pool(),
            sending.page_id,
            sending.attempt_id.unwrap(),
            &"f".repeat(64),
        )
        .await
        .unwrap_err();
        assert_eq!(late_old_attempt.code, AppErrorCode::RequestConflict);

        let second = recover_interrupted_pages(
            database.pool(),
            &scratch_root,
            now,
            Duration::from_secs(10 * 60),
        )
        .await
        .unwrap();
        assert_eq!(second.requeued, 0);
        assert_eq!(second.marked_retryable, 0);
        assert_eq!(second.unchanged_indexed, 1);
    });
}

#[derive(Debug, PartialEq, Eq)]
struct RecoveryRow {
    status: IndexPageStatus,
    attempt_id: Uuid,
    attempt_count: u32,
    has_render_hash: bool,
    safe_error_code: Option<String>,
    retryable: bool,
    content_version: u32,
    content_sha256: Option<String>,
    updated_at: String,
}

struct RecoveryFixture {
    pages: Vec<IndexPageOwnership>,
}

async fn create_recovery_fixture(pool: &SqlitePool, page_count: u32) -> RecoveryFixture {
    let timestamp = "2026-08-04T00:00:00.000Z";
    let book_id = Uuid::new_v4();
    let profile_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO books (id, sha256, title, format, original_filename, stored_path, import_status, created_at, updated_at) VALUES (?, ?, 'Synthetic recovery book', 'pdf', 'recovery.pdf', 'books/recovery/original.pdf', 'ready', ?, ?)",
    )
    .bind(book_id.to_string())
    .bind("a".repeat(64))
    .bind(timestamp)
    .bind(timestamp)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO provider_profiles (id, provider_kind, display_name, model_id, context_window_tokens, created_at, updated_at, validated_at) VALUES (?, 'openai', 'Synthetic recovery profile', 'synthetic-model', 32000, ?, ?, ?)",
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

    let mut pages = Vec::new();
    for page_number in 1..=page_count {
        pages.push(
            queue(pool, run_id, page_number, IndexQualityReason::NoText, None)
                .await
                .unwrap(),
        );
    }
    RecoveryFixture { pages }
}

async fn drive_to_sending(pool: &SqlitePool, page: IndexPageOwnership) {
    let attempt_id = page.attempt_id.unwrap();
    claim_render(pool, page.page_id, attempt_id).await.unwrap();
    mark_rendered(pool, page.page_id, attempt_id, &"a".repeat(64))
        .await
        .unwrap();
    claim_send(pool, page.page_id, attempt_id).await.unwrap();
}

async fn drive_to_parsing(pool: &SqlitePool, page: IndexPageOwnership) {
    drive_to_sending(pool, page).await;
    mark_received(
        pool,
        page.page_id,
        page.attempt_id.unwrap(),
        &"b".repeat(64),
    )
    .await
    .unwrap();
}

async fn drive_to_validating(pool: &SqlitePool, page: IndexPageOwnership) {
    drive_to_parsing(pool, page).await;
    claim_validate(pool, page.page_id, page.attempt_id.unwrap())
        .await
        .unwrap();
}

async fn set_updated_at(pool: &SqlitePool, page_id: Uuid, timestamp: &str) {
    sqlx::query("UPDATE index_pages SET updated_at = ? WHERE id = ?")
        .bind(timestamp)
        .bind(page_id.to_string())
        .execute(pool)
        .await
        .unwrap();
}

async fn page_recovery_row(pool: &SqlitePool, page_id: Uuid) -> RecoveryRow {
    let row = sqlx::query(
        "SELECT status, attempt_id, attempt_count, render_sha256, safe_error_code, retryable, content_version, content_sha256, updated_at FROM index_pages WHERE id = ?",
    )
    .bind(page_id.to_string())
    .fetch_one(pool)
    .await
    .unwrap();
    RecoveryRow {
        status: IndexPageStatus::from_database(&row.get::<String, _>("status")).unwrap(),
        attempt_id: Uuid::parse_str(&row.get::<String, _>("attempt_id")).unwrap(),
        attempt_count: u32::try_from(row.get::<i64, _>("attempt_count")).unwrap(),
        has_render_hash: row.get::<Option<String>, _>("render_sha256").is_some(),
        safe_error_code: row.get("safe_error_code"),
        retryable: row.get::<i64, _>("retryable") == 1,
        content_version: u32::try_from(row.get::<i64, _>("content_version")).unwrap(),
        content_sha256: row.get("content_sha256"),
        updated_at: row.get("updated_at"),
    }
}
