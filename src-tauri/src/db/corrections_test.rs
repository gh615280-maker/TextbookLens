use sqlx::{Row, SqlitePool};
use tokio_util::sync::CancellationToken;

use crate::{
    db::{
        Database,
        indexing::{CreateIndexRun, create_index_run},
    },
    domain::{
        ContentSource, IndexCorrectionConflictState, IndexCorrectionValueKind, IndexPageBlockKind,
        IndexQualityReason, NormalizedRect,
    },
    errors::AppErrorCode,
    indexing::{
        commit::{PageCommitRequest, commit_validated_page},
        state,
        validator::{ValidatedBlock, ValidatedPage},
    },
};

use super::*;

struct Fixture {
    _temporary: tempfile::TempDir,
    database: Database,
    book_id: Uuid,
    decoy_book_id: Uuid,
    page_id: Uuid,
    block_id: Uuid,
    original: String,
}

impl Fixture {
    fn new() -> Self {
        let temporary = tempfile::tempdir().unwrap();
        let database = Database::open(temporary.path().join("corrections.sqlite3")).unwrap();
        let (book_id, decoy_book_id, page_id, block_id, original) =
            tauri::async_runtime::block_on(async { create_fixture(database.pool()).await });
        Self {
            _temporary: temporary,
            database,
            book_id,
            decoy_book_id,
            page_id,
            block_id,
            original,
        }
    }

    fn pool(&self) -> &SqlitePool {
        self.database.pool()
    }

    fn save(&self, corrected_value: &str, expected_revision: u32) -> SaveIndexCorrection {
        SaveIndexCorrection {
            book_id: self.book_id,
            page_id: self.page_id,
            target_block_id: self.block_id,
            target_content_version: 1,
            value_kind: IndexCorrectionValueKind::Text,
            original_value_sha256: correction_value_sha256(&self.original),
            corrected_value: corrected_value.to_owned(),
            expected_revision,
        }
    }
}

#[test]
fn create_and_update_are_exact_revision_cas_and_refresh_the_overlay_fts_row() {
    let fixture = Fixture::new();
    tauri::async_runtime::block_on(async {
        let created =
            save_index_correction(fixture.pool(), fixture.save("corrected spectral law", 0))
                .await
                .unwrap();
        assert_eq!(created.revision, 1);
        assert_eq!(created.conflict_state, IndexCorrectionConflictState::Active);
        assert_eq!(created.original_value, fixture.original);
        assert_eq!(created.corrected_value, "corrected spectral law");

        let row = sqlx::query(
            "SELECT source, text, correction_id FROM index_search_chunks WHERE page_id = ? AND block_id = ? AND ordinal = 0",
        )
        .bind(fixture.page_id.to_string())
        .bind(fixture.block_id.to_string())
        .fetch_one(fixture.pool())
        .await
        .unwrap();
        assert_eq!(row.get::<String, _>("source"), "user_corrected");
        assert_eq!(row.get::<String, _>("text"), "corrected spectral law");
        assert_eq!(
            row.get::<Option<String>, _>("correction_id"),
            Some(created.id.to_string())
        );

        let updated = save_index_correction(
            fixture.pool(),
            fixture.save("corrected spectral law revision two", 1),
        )
        .await
        .unwrap();
        assert_eq!(updated.id, created.id);
        assert_eq!(updated.revision, 2);
        assert_eq!(
            updated.corrected_value,
            "corrected spectral law revision two"
        );

        let stale =
            save_index_correction(fixture.pool(), fixture.save("stale writer must lose", 1))
                .await
                .unwrap_err();
        assert_eq!(stale.code, AppErrorCode::RequestConflict);
        let persisted = list_page_corrections(fixture.pool(), fixture.page_id)
            .await
            .unwrap();
        assert_eq!(
            persisted[0].corrected_value,
            "corrected spectral law revision two"
        );
        assert_eq!(persisted[0].revision, 2);
    });
}

#[test]
fn invalid_empty_oversize_hash_version_and_cross_book_targets_are_rejected() {
    let fixture = Fixture::new();
    tauri::async_runtime::block_on(async {
        let mut empty = fixture.save("   ", 0);
        assert_eq!(
            save_index_correction(fixture.pool(), empty.clone())
                .await
                .unwrap_err()
                .code,
            AppErrorCode::InvalidInput
        );
        empty.corrected_value = "x".repeat(MAX_TEXT_CODE_POINTS + 1);
        assert_eq!(
            save_index_correction(fixture.pool(), empty.clone())
                .await
                .unwrap_err()
                .code,
            AppErrorCode::InvalidInput
        );
        empty.corrected_value = "bounded".to_owned();
        empty.original_value_sha256 = "f".repeat(64);
        assert_eq!(
            save_index_correction(fixture.pool(), empty.clone())
                .await
                .unwrap_err()
                .code,
            AppErrorCode::RequestConflict
        );
        empty.original_value_sha256 = correction_value_sha256(&fixture.original);
        empty.target_content_version = 2;
        assert_eq!(
            save_index_correction(fixture.pool(), empty.clone())
                .await
                .unwrap_err()
                .code,
            AppErrorCode::RequestConflict
        );
        empty.target_content_version = 1;
        empty.book_id = fixture.decoy_book_id;
        assert_eq!(
            save_index_correction(fixture.pool(), empty)
                .await
                .unwrap_err()
                .code,
            AppErrorCode::RequestConflict
        );
        assert!(
            list_page_corrections(fixture.pool(), fixture.page_id)
                .await
                .unwrap()
                .is_empty()
        );
    });
}

#[test]
fn delete_is_cas_and_atomically_reveals_the_current_provider_value() {
    let fixture = Fixture::new();
    tauri::async_runtime::block_on(async {
        let correction = save_index_correction(fixture.pool(), fixture.save("overlay sentinel", 0))
            .await
            .unwrap();
        let wrong_revision = delete_index_correction(
            fixture.pool(),
            DeleteIndexCorrection {
                book_id: fixture.book_id,
                page_id: fixture.page_id,
                correction_id: correction.id,
                target_content_version: 1,
                current_value_sha256: Some(correction_value_sha256(&fixture.original)),
                expected_revision: 2,
            },
        )
        .await
        .unwrap_err();
        assert_eq!(wrong_revision.code, AppErrorCode::RequestConflict);

        delete_index_correction(
            fixture.pool(),
            DeleteIndexCorrection {
                book_id: fixture.book_id,
                page_id: fixture.page_id,
                correction_id: correction.id,
                target_content_version: 1,
                current_value_sha256: Some(correction_value_sha256(&fixture.original)),
                expected_revision: 1,
            },
        )
        .await
        .unwrap();
        assert!(
            list_page_corrections(fixture.pool(), fixture.page_id)
                .await
                .unwrap()
                .is_empty()
        );
        let row = sqlx::query(
            "SELECT source, text, correction_id FROM index_search_chunks WHERE page_id = ? AND block_id = ? AND ordinal = 0",
        )
        .bind(fixture.page_id.to_string())
        .bind(fixture.block_id.to_string())
        .fetch_one(fixture.pool())
        .await
        .unwrap();
        assert_eq!(row.get::<String, _>("source"), "ai_transcribed");
        assert_eq!(row.get::<String, _>("text"), fixture.original);
        assert_eq!(row.get::<Option<String>, _>("correction_id"), None);
    });
}

async fn create_fixture(pool: &SqlitePool) -> (Uuid, Uuid, Uuid, Uuid, String) {
    let book_id = insert_book(pool, "Correction target", "a").await;
    let decoy_book_id = insert_book(pool, "Correction decoy", "b").await;
    let profile_id = Uuid::new_v4();
    let timestamp = "2026-08-04T00:00:00.000Z";
    sqlx::query(
        "INSERT INTO provider_profiles (id, provider_kind, display_name, model_id, context_window_tokens, created_at, updated_at, validated_at) VALUES (?, 'openai', 'Synthetic correction profile', 'gpt-5.6', 1050000, ?, ?, ?)",
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
            analysis_schema_version: "textbooklens.page-analysis.v1".to_owned(),
            render_version: "synthetic-render-v1".to_owned(),
            parser_version: "synthetic-parser-v1".to_owned(),
        },
    )
    .await
    .unwrap();
    let page = state::queue(pool, run_id, 1, IndexQualityReason::NoText, None)
        .await
        .unwrap();
    let attempt_id = page.attempt_id.unwrap();
    state::claim_render(pool, page.page_id, attempt_id)
        .await
        .unwrap();
    state::mark_rendered(pool, page.page_id, attempt_id, &"c".repeat(64))
        .await
        .unwrap();
    state::claim_send(pool, page.page_id, attempt_id)
        .await
        .unwrap();
    state::mark_received(pool, page.page_id, attempt_id, &"d".repeat(64))
        .await
        .unwrap();
    let original = "provider spectral law original".to_owned();
    let validated = ValidatedPage {
        page_number: 1,
        review_reason: None,
        blocks: vec![ValidatedBlock {
            ordinal: 0,
            kind: IndexPageBlockKind::Formula,
            plain_text: Some(original.clone()),
            latex: Some("E = K + U".to_owned()),
            table_cells: None,
            visual_description: Some("provider spectral law diagram".to_owned()),
            bounds: Some(NormalizedRect::new(0.1, 0.1, 0.8, 0.2).unwrap()),
            source: ContentSource::AiTranscribed,
        }],
    };
    commit_validated_page(
        pool,
        PageCommitRequest {
            page_id: page.page_id,
            attempt_id,
            page: &validated,
        },
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    let block_id: String =
        sqlx::query_scalar("SELECT id FROM index_page_blocks WHERE page_id = ? AND ordinal = 0")
            .bind(page.page_id.to_string())
            .fetch_one(pool)
            .await
            .unwrap();
    (
        book_id,
        decoy_book_id,
        page.page_id,
        Uuid::parse_str(&block_id).unwrap(),
        original,
    )
}

async fn insert_book(pool: &SqlitePool, title: &str, hash: &str) -> Uuid {
    let book_id = Uuid::new_v4();
    let timestamp = "2026-08-04T00:00:00.000Z";
    sqlx::query(
        "INSERT INTO books (id, sha256, title, format, original_filename, stored_path, import_status, created_at, updated_at) VALUES (?, ?, ?, 'pdf', 'synthetic.pdf', ?, 'ready', ?, ?)",
    )
    .bind(book_id.to_string())
    .bind(hash.repeat(64))
    .bind(title)
    .bind(format!("books/{book_id}/original.pdf"))
    .bind(timestamp)
    .bind(timestamp)
    .execute(pool)
    .await
    .unwrap();
    book_id
}
