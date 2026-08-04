use sqlx::{Row, SqlitePool};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{
    db::{
        Database,
        indexing::{CreateIndexRun, create_index_run},
    },
    domain::{
        ContentSource, IndexFailureCode, IndexPageBlockKind, IndexPageStatus, IndexQualityReason,
        IndexReviewReason, NormalizedRect, stable_index_page_block_id,
    },
    errors::AppErrorCode,
};

use super::{
    commit::{
        CommitBarrier, CommitControl, CommitFault, PageCommitRequest, commit_validated_page,
        commit_validated_page_with_control,
    },
    state,
    validator::{ValidatedBlock, ValidatedPage},
};

#[derive(Clone, Copy)]
struct PageAttempt {
    page_id: Uuid,
    attempt_id: Uuid,
}

struct Fixture {
    _temporary: tempfile::TempDir,
    database: Database,
    book_id: Uuid,
    run_id: Uuid,
    pages: Vec<PageAttempt>,
}

impl Fixture {
    fn new(page_count: u32) -> Self {
        let temporary = tempfile::tempdir().unwrap();
        let database = Database::open(temporary.path().join("commit.sqlite3")).unwrap();
        let (book_id, run_id, pages) = tauri::async_runtime::block_on(async {
            create_fixture(database.pool(), page_count).await
        });
        Self {
            _temporary: temporary,
            database,
            book_id,
            run_id,
            pages,
        }
    }

    fn pool(&self) -> &SqlitePool {
        self.database.pool()
    }
}

#[test]
fn one_page_transaction_writes_stable_blocks_chunks_fts_and_status_only_for_that_page() {
    let fixture = Fixture::new(2);
    tauri::async_runtime::block_on(async {
        let first = fixture.pages[0];
        let second = fixture.pages[1];
        let page = validated_page(1, "synthetic alpha energy passage", true);
        let outcome = commit_validated_page(
            fixture.pool(),
            PageCommitRequest {
                page_id: first.page_id,
                attempt_id: first.attempt_id,
                page: &page,
            },
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(outcome.status, IndexPageStatus::Indexed);
        assert_eq!(outcome.content_version, 1);
        assert_eq!(outcome.content_sha256.len(), 64);

        let page_row = sqlx::query(
            "SELECT status, content_version, review_reason_code FROM index_pages WHERE id = ?",
        )
        .bind(first.page_id.to_string())
        .fetch_one(fixture.pool())
        .await
        .unwrap();
        assert_eq!(page_row.get::<String, _>("status"), "indexed");
        assert_eq!(page_row.get::<i64, _>("content_version"), 1);
        assert_eq!(
            page_row.get::<Option<String>, _>("review_reason_code"),
            None
        );

        let block_id = stable_index_page_block_id(
            fixture.book_id,
            1,
            fixture.run_id,
            "textbooklens.page-analysis.v1",
            0,
        );
        let stored_block_id: String = sqlx::query_scalar(
            "SELECT id FROM index_page_blocks WHERE page_id = ? AND content_version = 1",
        )
        .bind(first.page_id.to_string())
        .fetch_one(fixture.pool())
        .await
        .unwrap();
        assert_eq!(stored_block_id, block_id.to_string());

        let sources: Vec<String> = sqlx::query_scalar(
            "SELECT source FROM index_search_chunks WHERE page_id = ? ORDER BY ordinal",
        )
        .bind(first.page_id.to_string())
        .fetch_all(fixture.pool())
        .await
        .unwrap();
        assert_eq!(sources, vec!["ai_transcribed", "ai_description"]);
        assert_eq!(fts_count(fixture.pool(), "alpha energy").await, 1);

        let second_row =
            sqlx::query("SELECT status, content_version FROM index_pages WHERE id = ?")
                .bind(second.page_id.to_string())
                .fetch_one(fixture.pool())
                .await
                .unwrap();
        assert_eq!(second_row.get::<String, _>("status"), "parsing");
        assert_eq!(second_row.get::<i64, _>("content_version"), 0);
        let second_blocks: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM index_page_blocks WHERE page_id = ?")
                .bind(second.page_id.to_string())
                .fetch_one(fixture.pool())
                .await
                .unwrap();
        assert_eq!(second_blocks, 0);
    });
}

#[test]
fn reanalysis_replaces_only_the_current_page_version() {
    let fixture = Fixture::new(2);
    tauri::async_runtime::block_on(async {
        commit_page(
            fixture.pool(),
            fixture.pages[0],
            &validated_page(1, "first version sentinel", false),
        )
        .await;
        commit_page(
            fixture.pool(),
            fixture.pages[1],
            &validated_page(2, "second page retained sentinel", false),
        )
        .await;

        let next_attempt = retry_to_parsing(fixture.pool(), fixture.pages[0]).await;
        commit_page(
            fixture.pool(),
            next_attempt,
            &validated_page(1, "replacement version sentinel", false),
        )
        .await;

        let first: (String, i64) = sqlx::query_as(
            "SELECT plain_text, content_version FROM index_page_blocks WHERE page_id = ?",
        )
        .bind(fixture.pages[0].page_id.to_string())
        .fetch_one(fixture.pool())
        .await
        .unwrap();
        assert_eq!(first, ("replacement version sentinel".to_owned(), 2));
        let second: (String, i64) = sqlx::query_as(
            "SELECT plain_text, content_version FROM index_page_blocks WHERE page_id = ?",
        )
        .bind(fixture.pages[1].page_id.to_string())
        .fetch_one(fixture.pool())
        .await
        .unwrap();
        assert_eq!(second, ("second page retained sentinel".to_owned(), 1));
        assert_eq!(fts_count(fixture.pool(), "first version").await, 0);
        assert_eq!(fts_count(fixture.pool(), "replacement version").await, 1);
        assert_eq!(fts_count(fixture.pool(), "second page retained").await, 1);
    });
}

#[test]
fn insert_fts_and_status_faults_roll_back_the_full_page_and_preserve_previous_search() {
    for fault in [
        CommitFault::BlockInsert,
        CommitFault::SearchInsert,
        CommitFault::StatusUpdate,
    ] {
        let fixture = Fixture::new(1);
        tauri::async_runtime::block_on(async {
            let first = fixture.pages[0];
            commit_page(
                fixture.pool(),
                first,
                &validated_page(1, "safe previous indexed version", false),
            )
            .await;
            let retry = retry_to_parsing(fixture.pool(), first).await;
            let replacement = validated_page(1, "fault replacement must vanish", false);
            let error = commit_validated_page_with_control(
                fixture.pool(),
                PageCommitRequest {
                    page_id: retry.page_id,
                    attempt_id: retry.attempt_id,
                    page: &replacement,
                },
                &CancellationToken::new(),
                &CommitControl::with_fault(fault),
            )
            .await
            .unwrap_err();
            assert_eq!(error.code, AppErrorCode::DatabaseError, "{fault:?}");

            let row = sqlx::query("SELECT status, content_version FROM index_pages WHERE id = ?")
                .bind(retry.page_id.to_string())
                .fetch_one(fixture.pool())
                .await
                .unwrap();
            assert_eq!(row.get::<String, _>("status"), "parsing", "{fault:?}");
            assert_eq!(row.get::<i64, _>("content_version"), 1, "{fault:?}");
            assert_eq!(
                fts_count(fixture.pool(), "safe previous indexed").await,
                1,
                "{fault:?}"
            );
            assert_eq!(
                fts_count(fixture.pool(), "fault replacement").await,
                0,
                "{fault:?}"
            );

            state::fail(
                fixture.pool(),
                retry.page_id,
                IndexPageStatus::Parsing,
                retry.attempt_id,
                IndexFailureCode::IndexValidationFailed,
                true,
            )
            .await
            .unwrap();
            assert_eq!(fts_count(fixture.pool(), "safe previous indexed").await, 1);
        });
    }
}

#[test]
fn cancellation_barriers_have_explicit_before_and_after_commit_semantics() {
    for barrier in [
        CommitBarrier::BeforeTransaction,
        CommitBarrier::BeforeStatusAndCommit,
    ] {
        let fixture = Fixture::new(1);
        tauri::async_runtime::block_on(async {
            let cancellation = CancellationToken::new();
            let observed = cancellation.clone();
            let control = CommitControl::with_barrier(move |current| {
                if current == barrier {
                    observed.cancel();
                }
            });
            let page = validated_page(1, "cancelled transaction sentinel", false);
            let error = commit_validated_page_with_control(
                fixture.pool(),
                PageCommitRequest {
                    page_id: fixture.pages[0].page_id,
                    attempt_id: fixture.pages[0].attempt_id,
                    page: &page,
                },
                &cancellation,
                &control,
            )
            .await
            .unwrap_err();
            assert_eq!(error.code, AppErrorCode::ImportCancelled);
            let row = sqlx::query("SELECT status, content_version FROM index_pages WHERE id = ?")
                .bind(fixture.pages[0].page_id.to_string())
                .fetch_one(fixture.pool())
                .await
                .unwrap();
            assert_eq!(row.get::<String, _>("status"), "parsing");
            assert_eq!(row.get::<i64, _>("content_version"), 0);
            assert_eq!(fts_count(fixture.pool(), "cancelled transaction").await, 0);
        });
    }

    let fixture = Fixture::new(1);
    tauri::async_runtime::block_on(async {
        let cancellation = CancellationToken::new();
        let observed = cancellation.clone();
        let control = CommitControl::with_barrier(move |barrier| {
            if barrier == CommitBarrier::AfterCommit {
                observed.cancel();
            }
        });
        let page = validated_page(1, "after commit cancellation wins nothing", false);
        let outcome = commit_validated_page_with_control(
            fixture.pool(),
            PageCommitRequest {
                page_id: fixture.pages[0].page_id,
                attempt_id: fixture.pages[0].attempt_id,
                page: &page,
            },
            &cancellation,
            &control,
        )
        .await
        .unwrap();
        assert!(cancellation.is_cancelled());
        assert_eq!(outcome.status, IndexPageStatus::Indexed);
        assert_eq!(
            fts_count(fixture.pool(), "after commit cancellation").await,
            1
        );
    });
}

#[test]
fn validation_finding_and_content_are_committed_together() {
    let fixture = Fixture::new(1);
    tauri::async_runtime::block_on(async {
        let mut page = validated_page(1, "review content sentinel", false);
        page.review_reason = Some(IndexReviewReason::TextContradiction);
        let outcome = commit_page(fixture.pool(), fixture.pages[0], &page).await;
        assert_eq!(outcome.status, IndexPageStatus::NeedsReview);
        let reason: String =
            sqlx::query_scalar("SELECT review_reason_code FROM index_pages WHERE id = ?")
                .bind(fixture.pages[0].page_id.to_string())
                .fetch_one(fixture.pool())
                .await
                .unwrap();
        assert_eq!(reason, "text_contradiction");
        assert_eq!(fts_count(fixture.pool(), "review content").await, 1);
    });
}

async fn create_fixture(pool: &SqlitePool, page_count: u32) -> (Uuid, Uuid, Vec<PageAttempt>) {
    let book_id = Uuid::new_v4();
    let profile_id = Uuid::new_v4();
    let timestamp = "2026-08-04T00:00:00.000Z";
    sqlx::query(
        "INSERT INTO books (id, sha256, title, format, original_filename, stored_path, import_status, created_at, updated_at) VALUES (?, ?, 'Synthetic commit book', 'pdf', 'synthetic.pdf', ?, 'ready', ?, ?)",
    )
    .bind(book_id.to_string())
    .bind("a".repeat(64))
    .bind(format!("books/{book_id}/original.pdf"))
    .bind(timestamp)
    .bind(timestamp)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO provider_profiles (id, provider_kind, display_name, model_id, context_window_tokens, created_at, updated_at, validated_at) VALUES (?, 'openai', 'Synthetic commit profile', 'gpt-5.6', 1050000, ?, ?, ?)",
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
    let mut pages = Vec::new();
    for page_number in 1..=page_count {
        let page = state::queue(pool, run_id, page_number, IndexQualityReason::NoText, None)
            .await
            .unwrap();
        let attempt_id = page.attempt_id.unwrap();
        drive_to_parsing(pool, page.page_id, attempt_id).await;
        pages.push(PageAttempt {
            page_id: page.page_id,
            attempt_id,
        });
    }
    (book_id, run_id, pages)
}

async fn drive_to_parsing(pool: &SqlitePool, page_id: Uuid, attempt_id: Uuid) {
    state::claim_render(pool, page_id, attempt_id)
        .await
        .unwrap();
    state::mark_rendered(pool, page_id, attempt_id, &"b".repeat(64))
        .await
        .unwrap();
    state::claim_send(pool, page_id, attempt_id).await.unwrap();
    state::mark_received(pool, page_id, attempt_id, &"c".repeat(64))
        .await
        .unwrap();
}

async fn retry_to_parsing(pool: &SqlitePool, previous: PageAttempt) -> PageAttempt {
    let next = state::retry(
        pool,
        previous.page_id,
        IndexPageStatus::Indexed,
        previous.attempt_id,
    )
    .await
    .unwrap();
    drive_to_parsing(pool, previous.page_id, next).await;
    PageAttempt {
        page_id: previous.page_id,
        attempt_id: next,
    }
}

async fn commit_page(
    pool: &SqlitePool,
    attempt: PageAttempt,
    page: &ValidatedPage,
) -> super::commit::PageCommitOutcome {
    commit_validated_page(
        pool,
        PageCommitRequest {
            page_id: attempt.page_id,
            attempt_id: attempt.attempt_id,
            page,
        },
        &CancellationToken::new(),
    )
    .await
    .unwrap()
}

fn validated_page(page_number: u32, text: &str, description: bool) -> ValidatedPage {
    ValidatedPage {
        page_number,
        review_reason: None,
        blocks: vec![ValidatedBlock {
            ordinal: 0,
            kind: IndexPageBlockKind::Paragraph,
            plain_text: Some(text.to_owned()),
            latex: None,
            table_cells: None,
            visual_description: description
                .then(|| "synthetic auxiliary diagram description".to_owned()),
            bounds: Some(NormalizedRect::new(0.1, 0.1, 0.8, 0.2).unwrap()),
            source: ContentSource::AiTranscribed,
        }],
    }
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
