use sqlx::SqlitePool;
use textbooklens_lib::{
    db::{
        Database,
        corrections::{
            CorrectionConflictDecision, DeleteIndexCorrection, ResolveIndexCorrectionConflict,
            SaveIndexCorrection, correction_value_sha256, delete_index_correction,
            list_page_corrections, resolve_index_correction_conflict, save_index_correction,
        },
        indexing::{CreateIndexRun, create_index_run},
    },
    domain::{
        ContentSource, DocumentLocator, IndexCorrectionConflictState, IndexCorrectionValueKind,
        IndexPageBlockKind, IndexPageStatus, IndexQualityReason, NormalizedRect,
    },
    errors::AppErrorCode,
    indexing::{
        commit::{PageCommitRequest, commit_validated_page},
        state,
        validator::{ValidatedBlock, ValidatedPage},
    },
    retrieval::search::search_book,
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

struct TestContext {
    _temporary: tempfile::TempDir,
    database: Database,
    profile_id: Uuid,
}

impl TestContext {
    fn new() -> Self {
        let temporary = tempfile::tempdir().unwrap();
        let database = Database::open(temporary.path().join("index-corrections.sqlite3")).unwrap();
        let profile_id =
            tauri::async_runtime::block_on(async { insert_profile(database.pool()).await });
        Self {
            _temporary: temporary,
            database,
            profile_id,
        }
    }

    fn pool(&self) -> &SqlitePool {
        self.database.pool()
    }
}

struct IndexedPage {
    book_id: Uuid,
    page_id: Uuid,
    attempt_id: Uuid,
    block_id: Uuid,
    content_version: u32,
    provider_value: String,
}

#[test]
fn unchanged_reanalysis_retains_correction_changed_target_conflicts_and_all_decisions_are_atomic() {
    let context = TestContext::new();
    tauri::async_runtime::block_on(async {
        let book_id = insert_book(context.pool(), "Conflict decisions", "a").await;
        let mut page = create_indexed_page(
            context.pool(),
            book_id,
            context.profile_id,
            "provider alpha original",
            "unrelated auxiliary diagram",
        )
        .await;
        let created =
            save_text_correction(context.pool(), &page, "human overlay alpha retained", 0).await;
        assert_eq!(created.revision, 1);

        reanalyze_page(context.pool(), &mut page, "provider alpha original").await;
        let unchanged = only_correction(context.pool(), page.page_id).await;
        assert_eq!(unchanged.id, created.id);
        assert_eq!(unchanged.revision, 2);
        assert_eq!(
            unchanged.conflict_state,
            IndexCorrectionConflictState::Active
        );
        assert_eq!(
            user_source_count(context.pool(), book_id, "human overlay alpha").await,
            1
        );

        reanalyze_page(context.pool(), &mut page, "ambiguous provider gamma value").await;
        let conflicted = only_correction(context.pool(), page.page_id).await;
        assert_eq!(conflicted.revision, 3);
        assert_eq!(
            conflicted.conflict_state,
            IndexCorrectionConflictState::Conflict
        );
        assert!(
            search_book(context.pool(), book_id, "ambiguous provider gamma", 20)
                .await
                .unwrap()
                .is_empty(),
            "the ambiguous new provider value must be excluded"
        );
        assert!(
            search_book(context.pool(), book_id, "human overlay alpha", 20)
                .await
                .unwrap()
                .is_empty(),
            "the old overlay must also be excluded while conflicted"
        );

        let kept = resolve_index_correction_conflict(
            context.pool(),
            ResolveIndexCorrectionConflict {
                book_id,
                page_id: page.page_id,
                correction_id: conflicted.id,
                target_content_version: page.content_version,
                current_value_sha256: Some(correction_value_sha256(&page.provider_value)),
                expected_revision: conflicted.revision,
                decision: CorrectionConflictDecision::Keep,
                compared_corrected_value: None,
            },
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(kept.revision, 4);
        assert_eq!(kept.conflict_state, IndexCorrectionConflictState::Active);
        assert_eq!(
            user_source_count(context.pool(), book_id, "human overlay alpha").await,
            1
        );

        reanalyze_page(context.pool(), &mut page, "accepted provider delta value").await;
        let accept_conflict = only_correction(context.pool(), page.page_id).await;
        assert_eq!(accept_conflict.revision, 5);
        let accepted = resolve_index_correction_conflict(
            context.pool(),
            ResolveIndexCorrectionConflict {
                book_id,
                page_id: page.page_id,
                correction_id: accept_conflict.id,
                target_content_version: page.content_version,
                current_value_sha256: Some(correction_value_sha256(&page.provider_value)),
                expected_revision: accept_conflict.revision,
                decision: CorrectionConflictDecision::Accept,
                compared_corrected_value: None,
            },
        )
        .await
        .unwrap();
        assert!(accepted.is_none());
        assert!(
            list_page_corrections(context.pool(), page.page_id)
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            source_count(
                context.pool(),
                book_id,
                "accepted provider delta",
                ContentSource::AiTranscribed,
            )
            .await,
            1
        );

        let replacement =
            save_text_correction(context.pool(), &page, "temporary human comparison", 0).await;
        reanalyze_page(context.pool(), &mut page, "compared provider epsilon value").await;
        let compare_conflict = only_correction(context.pool(), page.page_id).await;
        assert_eq!(compare_conflict.id, replacement.id);
        assert_eq!(compare_conflict.revision, 2);
        let compared = resolve_index_correction_conflict(
            context.pool(),
            ResolveIndexCorrectionConflict {
                book_id,
                page_id: page.page_id,
                correction_id: compare_conflict.id,
                target_content_version: page.content_version,
                current_value_sha256: Some(correction_value_sha256(&page.provider_value)),
                expected_revision: compare_conflict.revision,
                decision: CorrectionConflictDecision::Compare,
                compared_corrected_value: Some("comparison saved human epsilon".to_owned()),
            },
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(compared.revision, 3);
        assert_eq!(compared.original_value, "compared provider epsilon value");
        assert_eq!(compared.corrected_value, "comparison saved human epsilon");
        assert_eq!(
            user_source_count(context.pool(), book_id, "comparison saved human epsilon").await,
            1
        );
    });
}

#[test]
fn retrieval_preserves_same_wording_sources_correction_audit_and_more_relevant_decoy_isolation() {
    let context = TestContext::new();
    tauri::async_runtime::block_on(async {
        let book_id = insert_book(context.pool(), "Provenance target", "b").await;
        let decoy_id = insert_book(context.pool(), "More relevant decoy", "c").await;
        insert_local_chunk(context.pool(), book_id, "shared provenance phrase").await;
        insert_local_chunk(
            context.pool(),
            decoy_id,
            &format!(
                "{} decoy provenance leak marker",
                "shared provenance phrase ".repeat(100)
            ),
        )
        .await;
        let page = create_indexed_page(
            context.pool(),
            book_id,
            context.profile_id,
            "shared provenance phrase",
            "shared provenance phrase",
        )
        .await;

        let before = search_book(context.pool(), book_id, "shared provenance phrase", 20)
            .await
            .unwrap();
        assert_eq!(before.len(), 3);
        assert!(
            before
                .iter()
                .all(|hit| !hit.snippet.contains("decoy provenance leak marker"))
        );
        assert_eq!(
            before
                .iter()
                .filter(|hit| hit.provenance.source == ContentSource::LocalText)
                .count(),
            1
        );
        assert_eq!(
            before
                .iter()
                .filter(|hit| hit.provenance.source == ContentSource::AiTranscribed)
                .count(),
            1
        );
        let description = before
            .iter()
            .find(|hit| hit.provenance.source == ContentSource::AiDescription)
            .unwrap();
        assert!(!description.provenance.quoteable);

        let correction =
            save_text_correction(context.pool(), &page, "shared provenance phrase", 0).await;
        let after = search_book(context.pool(), book_id, "shared provenance phrase", 20)
            .await
            .unwrap();
        assert_eq!(
            after.len(),
            3,
            "identical text must not be source-deduplicated"
        );
        let corrected = after
            .iter()
            .find(|hit| hit.provenance.source == ContentSource::UserCorrected)
            .unwrap();
        assert_eq!(corrected.provenance.correction_id, Some(correction.id));
        assert_eq!(
            corrected.provenance.original_source,
            Some(ContentSource::AiTranscribed)
        );
        assert_eq!(
            corrected.provenance.original_value_sha256,
            Some(correction_value_sha256(&page.provider_value))
        );
        assert!(corrected.provenance.quoteable);
        assert!(
            after
                .iter()
                .all(|hit| !hit.snippet.contains("decoy provenance leak marker"))
        );
    });
}

#[test]
fn deletion_is_exact_cas_reveals_provider_and_cross_book_or_stale_targets_reject() {
    let context = TestContext::new();
    tauri::async_runtime::block_on(async {
        let book_id = insert_book(context.pool(), "Delete correction", "d").await;
        let other_book_id = insert_book(context.pool(), "Cross-book decoy", "e").await;
        let page = create_indexed_page(
            context.pool(),
            book_id,
            context.profile_id,
            "provider deletion reveal sentinel",
            "unrelated auxiliary description",
        )
        .await;
        let cross_book = save_index_correction(
            context.pool(),
            SaveIndexCorrection {
                book_id: other_book_id,
                page_id: page.page_id,
                target_block_id: page.block_id,
                target_content_version: page.content_version,
                value_kind: IndexCorrectionValueKind::Text,
                original_value_sha256: correction_value_sha256(&page.provider_value),
                corrected_value: "cross-book rejected".to_owned(),
                expected_revision: 0,
            },
        )
        .await
        .unwrap_err();
        assert_eq!(cross_book.code, AppErrorCode::RequestConflict);
        let stale_version = save_index_correction(
            context.pool(),
            SaveIndexCorrection {
                book_id,
                page_id: page.page_id,
                target_block_id: page.block_id,
                target_content_version: page.content_version + 1,
                value_kind: IndexCorrectionValueKind::Text,
                original_value_sha256: correction_value_sha256(&page.provider_value),
                corrected_value: "stale version rejected".to_owned(),
                expected_revision: 0,
            },
        )
        .await
        .unwrap_err();
        assert_eq!(stale_version.code, AppErrorCode::RequestConflict);

        let correction =
            save_text_correction(context.pool(), &page, "deletion overlay sentinel", 0).await;
        let stale_revision = delete_index_correction(
            context.pool(),
            DeleteIndexCorrection {
                book_id,
                page_id: page.page_id,
                correction_id: correction.id,
                target_content_version: page.content_version,
                current_value_sha256: Some(correction_value_sha256(&page.provider_value)),
                expected_revision: correction.revision + 1,
            },
        )
        .await
        .unwrap_err();
        assert_eq!(stale_revision.code, AppErrorCode::RequestConflict);
        delete_index_correction(
            context.pool(),
            DeleteIndexCorrection {
                book_id,
                page_id: page.page_id,
                correction_id: correction.id,
                target_content_version: page.content_version,
                current_value_sha256: Some(correction_value_sha256(&page.provider_value)),
                expected_revision: correction.revision,
            },
        )
        .await
        .unwrap();
        assert!(
            search_book(context.pool(), book_id, "deletion overlay sentinel", 20)
                .await
                .unwrap()
                .is_empty()
        );
        let revealed = search_book(context.pool(), book_id, "provider deletion reveal", 20)
            .await
            .unwrap();
        assert_eq!(revealed.len(), 1);
        assert_eq!(revealed[0].provenance.source, ContentSource::AiTranscribed);
        assert!(revealed[0].provenance.correction_id.is_none());
    });
}

#[test]
fn page_and_book_deletion_cascade_corrections_blocks_chunks_and_fts() {
    let context = TestContext::new();
    tauri::async_runtime::block_on(async {
        let page_book = insert_book(context.pool(), "Page cascade", "f").await;
        let page = create_indexed_page(
            context.pool(),
            page_book,
            context.profile_id,
            "page cascade provider sentinel",
            "page cascade description",
        )
        .await;
        save_text_correction(context.pool(), &page, "page cascade correction sentinel", 0).await;
        sqlx::query("DELETE FROM index_pages WHERE id = ?")
            .bind(page.page_id.to_string())
            .execute(context.pool())
            .await
            .unwrap();
        assert_eq!(
            count_for_page(context.pool(), "index_corrections", page.page_id).await,
            0
        );
        assert_eq!(
            count_for_page(context.pool(), "index_page_blocks", page.page_id).await,
            0
        );
        assert_eq!(
            count_for_page(context.pool(), "index_search_chunks", page.page_id).await,
            0
        );
        assert_eq!(
            fts_count(context.pool(), "page cascade correction").await,
            0
        );

        let book_id = insert_book(context.pool(), "Book cascade", "1").await;
        let book_page = create_indexed_page(
            context.pool(),
            book_id,
            context.profile_id,
            "book cascade provider sentinel",
            "book cascade description",
        )
        .await;
        save_text_correction(
            context.pool(),
            &book_page,
            "book cascade correction sentinel",
            0,
        )
        .await;
        sqlx::query("DELETE FROM books WHERE id = ?")
            .bind(book_id.to_string())
            .execute(context.pool())
            .await
            .unwrap();
        for table in [
            "index_pages",
            "index_page_blocks",
            "index_search_chunks",
            "index_corrections",
        ] {
            assert_eq!(
                count_for_book(context.pool(), table, book_id).await,
                0,
                "{table}"
            );
        }
        assert_eq!(
            fts_count(context.pool(), "book cascade correction").await,
            0
        );
    });
}

async fn insert_profile(pool: &SqlitePool) -> Uuid {
    let profile_id = Uuid::new_v4();
    let timestamp = "2026-08-04T00:00:00.000Z";
    sqlx::query(
        "INSERT INTO provider_profiles (id, provider_kind, display_name, model_id, context_window_tokens, created_at, updated_at, validated_at) VALUES (?, 'openai', 'Synthetic integration profile', 'gpt-5.6', 1050000, ?, ?, ?)",
    )
    .bind(profile_id.to_string())
    .bind(timestamp)
    .bind(timestamp)
    .bind(timestamp)
    .execute(pool)
    .await
    .unwrap();
    profile_id
}

async fn insert_book(pool: &SqlitePool, title: &str, hash_character: &str) -> Uuid {
    let book_id = Uuid::new_v4();
    let timestamp = "2026-08-04T00:00:00.000Z";
    sqlx::query(
        "INSERT INTO books (id, sha256, title, format, original_filename, stored_path, import_status, created_at, updated_at) VALUES (?, ?, ?, 'pdf', 'synthetic.pdf', ?, 'ready', ?, ?)",
    )
    .bind(book_id.to_string())
    .bind(hash_character.repeat(64))
    .bind(title)
    .bind(format!("books/{book_id}/original.pdf"))
    .bind(timestamp)
    .bind(timestamp)
    .execute(pool)
    .await
    .unwrap();
    book_id
}

async fn create_indexed_page(
    pool: &SqlitePool,
    book_id: Uuid,
    profile_id: Uuid,
    provider_value: &str,
    description: &str,
) -> IndexedPage {
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
    drive_to_parsing(pool, page.page_id, attempt_id).await;
    commit_provider_value(pool, page.page_id, attempt_id, provider_value, description).await;
    let block_id: String = sqlx::query_scalar(
        "SELECT id FROM index_page_blocks WHERE page_id = ? AND content_version = 1 AND ordinal = 0",
    )
    .bind(page.page_id.to_string())
    .fetch_one(pool)
    .await
    .unwrap();
    IndexedPage {
        book_id,
        page_id: page.page_id,
        attempt_id,
        block_id: Uuid::parse_str(&block_id).unwrap(),
        content_version: 1,
        provider_value: provider_value.to_owned(),
    }
}

async fn reanalyze_page(pool: &SqlitePool, page: &mut IndexedPage, provider_value: &str) {
    let attempt_id = state::retry(
        pool,
        page.page_id,
        IndexPageStatus::Indexed,
        page.attempt_id,
    )
    .await
    .unwrap();
    drive_to_parsing(pool, page.page_id, attempt_id).await;
    commit_provider_value(
        pool,
        page.page_id,
        attempt_id,
        provider_value,
        "unrelated auxiliary diagram",
    )
    .await;
    page.attempt_id = attempt_id;
    page.content_version += 1;
    page.provider_value = provider_value.to_owned();
}

async fn drive_to_parsing(pool: &SqlitePool, page_id: Uuid, attempt_id: Uuid) {
    state::claim_render(pool, page_id, attempt_id)
        .await
        .unwrap();
    state::mark_rendered(pool, page_id, attempt_id, &"2".repeat(64))
        .await
        .unwrap();
    state::claim_send(pool, page_id, attempt_id).await.unwrap();
    state::mark_received(pool, page_id, attempt_id, &"3".repeat(64))
        .await
        .unwrap();
}

async fn commit_provider_value(
    pool: &SqlitePool,
    page_id: Uuid,
    attempt_id: Uuid,
    provider_value: &str,
    description: &str,
) {
    let page = ValidatedPage {
        page_number: 1,
        review_reason: None,
        blocks: vec![ValidatedBlock {
            ordinal: 0,
            kind: IndexPageBlockKind::Paragraph,
            plain_text: Some(provider_value.to_owned()),
            latex: Some("E = K + U".to_owned()),
            table_cells: None,
            visual_description: Some(description.to_owned()),
            bounds: Some(NormalizedRect::new(0.1, 0.1, 0.8, 0.2).unwrap()),
            source: ContentSource::AiTranscribed,
        }],
    };
    commit_validated_page(
        pool,
        PageCommitRequest {
            page_id,
            attempt_id,
            page: &page,
        },
        &CancellationToken::new(),
    )
    .await
    .unwrap();
}

async fn save_text_correction(
    pool: &SqlitePool,
    page: &IndexedPage,
    corrected_value: &str,
    expected_revision: u32,
) -> textbooklens_lib::domain::IndexCorrectionReviewDto {
    save_index_correction(
        pool,
        SaveIndexCorrection {
            book_id: page.book_id,
            page_id: page.page_id,
            target_block_id: page.block_id,
            target_content_version: page.content_version,
            value_kind: IndexCorrectionValueKind::Text,
            original_value_sha256: correction_value_sha256(&page.provider_value),
            corrected_value: corrected_value.to_owned(),
            expected_revision,
        },
    )
    .await
    .unwrap()
}

async fn only_correction(
    pool: &SqlitePool,
    page_id: Uuid,
) -> textbooklens_lib::domain::IndexCorrectionReviewDto {
    let corrections = list_page_corrections(pool, page_id).await.unwrap();
    assert_eq!(corrections.len(), 1);
    corrections.into_iter().next().unwrap()
}

async fn insert_local_chunk(pool: &SqlitePool, book_id: Uuid, text: &str) {
    let section_id = Uuid::new_v4();
    let locator = serde_json::to_string(&DocumentLocator::pdf(1, 1, None).unwrap()).unwrap();
    sqlx::query(
        "INSERT INTO sections (id, book_id, ordinal, title, locator_json) VALUES (?, ?, 0, 'Synthetic local source', ?)",
    )
    .bind(section_id.to_string())
    .bind(book_id.to_string())
    .bind(&locator)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO search_chunks (id, book_id, section_id, ordinal, text, locator_json, token_estimate) VALUES (?, ?, ?, 0, ?, ?, 10)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(book_id.to_string())
    .bind(section_id.to_string())
    .bind(text)
    .bind(locator)
    .execute(pool)
    .await
    .unwrap();
}

async fn source_count(
    pool: &SqlitePool,
    book_id: Uuid,
    query: &str,
    source: ContentSource,
) -> usize {
    search_book(pool, book_id, query, 20)
        .await
        .unwrap()
        .into_iter()
        .filter(|hit| hit.provenance.source == source)
        .count()
}

async fn user_source_count(pool: &SqlitePool, book_id: Uuid, query: &str) -> usize {
    source_count(pool, book_id, query, ContentSource::UserCorrected).await
}

async fn count_for_page(pool: &SqlitePool, table: &str, page_id: Uuid) -> i64 {
    let query = match table {
        "index_corrections" => "SELECT COUNT(*) FROM index_corrections WHERE page_id = ?",
        "index_page_blocks" => "SELECT COUNT(*) FROM index_page_blocks WHERE page_id = ?",
        "index_search_chunks" => "SELECT COUNT(*) FROM index_search_chunks WHERE page_id = ?",
        _ => panic!("unexpected synthetic page table"),
    };
    sqlx::query_scalar(query)
        .bind(page_id.to_string())
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn count_for_book(pool: &SqlitePool, table: &str, book_id: Uuid) -> i64 {
    let query = match table {
        "index_pages" => "SELECT COUNT(*) FROM index_pages WHERE book_id = ?",
        "index_page_blocks" => "SELECT COUNT(*) FROM index_page_blocks WHERE book_id = ?",
        "index_search_chunks" => "SELECT COUNT(*) FROM index_search_chunks WHERE book_id = ?",
        "index_corrections" => "SELECT COUNT(*) FROM index_corrections WHERE book_id = ?",
        _ => panic!("unexpected synthetic book table"),
    };
    sqlx::query_scalar(query)
        .bind(book_id.to_string())
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn fts_count(pool: &SqlitePool, phrase: &str) -> i64 {
    let query = format!("\"{}\"", phrase.replace('"', "\"\""));
    sqlx::query_scalar(
        "SELECT COUNT(*) FROM index_search_chunks_fts WHERE index_search_chunks_fts MATCH ?",
    )
    .bind(query)
    .fetch_one(pool)
    .await
    .unwrap()
}
