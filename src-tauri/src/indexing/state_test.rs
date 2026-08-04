use std::sync::Arc;

use sqlx::{Row, SqlitePool};
use tokio::sync::Barrier;
use uuid::Uuid;

use crate::{
    db::{
        Database,
        indexing::{CreateIndexRun, IndexPageOwnership, create_index_run, get_run_aggregate},
    },
    domain::{
        IndexAggregateStatus, IndexFailureCode, IndexPageStatus, IndexPageStatusCountsDto,
        IndexQualityReason, IndexReviewReason, IndexRunStatus,
    },
    errors::AppErrorCode,
};

use super::state::{
    IndexCancellationRegistry, cancel, cancel_run, claim_render, claim_send, claim_validate,
    commit_indexed, commit_review, fail, finalize_run_if_terminal, legal_transition, mark_received,
    mark_rendered, pause, queue, resume, retry,
};

#[test]
fn transition_table_and_terminal_retry_are_exact_attempt_owned() {
    let cases = [
        (
            IndexPageStatus::Queued,
            IndexPageStatus::Rendering,
            false,
            true,
        ),
        (
            IndexPageStatus::Rendering,
            IndexPageStatus::Sending,
            false,
            true,
        ),
        (
            IndexPageStatus::Sending,
            IndexPageStatus::Parsing,
            false,
            true,
        ),
        (
            IndexPageStatus::Parsing,
            IndexPageStatus::Validating,
            false,
            true,
        ),
        (
            IndexPageStatus::Validating,
            IndexPageStatus::Indexed,
            false,
            true,
        ),
        (
            IndexPageStatus::Indexed,
            IndexPageStatus::Queued,
            false,
            false,
        ),
        (
            IndexPageStatus::Indexed,
            IndexPageStatus::Queued,
            true,
            true,
        ),
        (
            IndexPageStatus::Failed,
            IndexPageStatus::Rendering,
            false,
            false,
        ),
        (
            IndexPageStatus::NotRequired,
            IndexPageStatus::Queued,
            true,
            false,
        ),
    ];
    for (from, to, explicit_retry, expected) in cases {
        assert_eq!(
            legal_transition(from, to, explicit_retry),
            expected,
            "{from:?} -> {to:?}, retry={explicit_retry}"
        );
    }

    let temporary = tempfile::tempdir().unwrap();
    let database = Database::open(temporary.path().join("state-transition.sqlite3")).unwrap();
    tauri::async_runtime::block_on(async {
        let fixture = create_state_fixture(database.pool(), 1).await;
        let page = fixture.pages[0];
        let first_attempt = page.attempt_id.unwrap();
        drive_to_validating(database.pool(), page.page_id, first_attempt).await;
        commit_indexed(
            database.pool(),
            page.page_id,
            first_attempt,
            &"c".repeat(64),
        )
        .await
        .unwrap();

        let terminal_regression = fail(
            database.pool(),
            page.page_id,
            IndexPageStatus::Indexed,
            first_attempt,
            IndexFailureCode::IndexValidationFailed,
            true,
        )
        .await
        .unwrap_err();
        assert_eq!(terminal_regression.code, AppErrorCode::RequestConflict);

        let second_attempt = retry(
            database.pool(),
            page.page_id,
            IndexPageStatus::Indexed,
            first_attempt,
        )
        .await
        .unwrap();
        assert_ne!(second_attempt, first_attempt);
        let late_old_attempt = claim_render(database.pool(), page.page_id, first_attempt)
            .await
            .unwrap_err();
        assert_eq!(late_old_attempt.code, AppErrorCode::RequestConflict);
        claim_render(database.pool(), page.page_id, second_attempt)
            .await
            .unwrap();
    });
}

#[test]
fn two_barrier_released_coordinators_cannot_both_commit() {
    let temporary = tempfile::tempdir().unwrap();
    let database = Database::open(temporary.path().join("state-barrier.sqlite3")).unwrap();
    tauri::async_runtime::block_on(async {
        let fixture = create_state_fixture(database.pool(), 1).await;
        let page = fixture.pages[0];
        let attempt_id = page.attempt_id.unwrap();
        drive_to_validating(database.pool(), page.page_id, attempt_id).await;

        let barrier = Arc::new(Barrier::new(3));
        let first_pool = database.pool().clone();
        let first_barrier = Arc::clone(&barrier);
        let first = async move {
            first_barrier.wait().await;
            commit_indexed(&first_pool, page.page_id, attempt_id, &"d".repeat(64)).await
        };
        let second_pool = database.pool().clone();
        let second_barrier = Arc::clone(&barrier);
        let second = async move {
            second_barrier.wait().await;
            commit_review(
                &second_pool,
                page.page_id,
                attempt_id,
                IndexReviewReason::IncompleteContent,
                &"e".repeat(64),
            )
            .await
        };
        let release = barrier.wait();
        let (first, second, _) = tokio::join!(first, second, release);
        let outcomes = [first, second];
        assert_eq!(outcomes.iter().filter(|result| result.is_ok()).count(), 1);
        let loser = outcomes
            .iter()
            .find_map(|result| result.as_ref().err())
            .unwrap();
        assert_eq!(loser.code, AppErrorCode::RequestConflict);
        assert!(matches!(
            page_status(database.pool(), page.page_id).await,
            IndexPageStatus::Indexed | IndexPageStatus::NeedsReview
        ));
    });
}

#[test]
fn run_aggregate_is_derived_from_page_truth_for_mixed_outcomes() {
    let table = [
        (
            vec![IndexPageStatus::NotRequired, IndexPageStatus::Indexed],
            IndexAggregateStatus::Ready,
        ),
        (
            vec![IndexPageStatus::Indexed, IndexPageStatus::NeedsReview],
            IndexAggregateStatus::NeedsReview,
        ),
        (
            vec![IndexPageStatus::Indexed, IndexPageStatus::Failed],
            IndexAggregateStatus::Partial,
        ),
        (
            vec![IndexPageStatus::NeedsReview, IndexPageStatus::Failed],
            IndexAggregateStatus::Partial,
        ),
        (
            vec![IndexPageStatus::Failed, IndexPageStatus::Cancelled],
            IndexAggregateStatus::Failed,
        ),
        (vec![IndexPageStatus::Queued], IndexAggregateStatus::Partial),
    ];
    for (statuses, expected) in table {
        let mut counts = IndexPageStatusCountsDto::default();
        for status in statuses {
            counts.record(status);
        }
        assert_eq!(counts.aggregate_status(), expected);
    }

    let temporary = tempfile::tempdir().unwrap();
    let database = Database::open(temporary.path().join("state-aggregate.sqlite3")).unwrap();
    tauri::async_runtime::block_on(async {
        let fixture = create_state_fixture(database.pool(), 3).await;
        let indexed = fixture.pages[0];
        let review = fixture.pages[1];
        let failed = fixture.pages[2];
        drive_to_validating(
            database.pool(),
            indexed.page_id,
            indexed.attempt_id.unwrap(),
        )
        .await;
        commit_indexed(
            database.pool(),
            indexed.page_id,
            indexed.attempt_id.unwrap(),
            &"1".repeat(64),
        )
        .await
        .unwrap();
        drive_to_validating(database.pool(), review.page_id, review.attempt_id.unwrap()).await;
        commit_review(
            database.pool(),
            review.page_id,
            review.attempt_id.unwrap(),
            IndexReviewReason::TextContradiction,
            &"2".repeat(64),
        )
        .await
        .unwrap();
        fail(
            database.pool(),
            failed.page_id,
            IndexPageStatus::Queued,
            failed.attempt_id.unwrap(),
            IndexFailureCode::IndexProviderFailed,
            true,
        )
        .await
        .unwrap();

        let aggregate = get_run_aggregate(database.pool(), fixture.run_id)
            .await
            .unwrap();
        assert_eq!(aggregate.pages.indexed, 1);
        assert_eq!(aggregate.pages.needs_review, 1);
        assert_eq!(aggregate.pages.failed, 1);
        assert_eq!(aggregate.aggregate_status, IndexAggregateStatus::Partial);
    });
}

#[test]
fn pause_allows_inflight_terminal_cancel_signals_active_and_cancels_queued() {
    let temporary = tempfile::tempdir().unwrap();
    let database = Database::open(temporary.path().join("state-control.sqlite3")).unwrap();
    tauri::async_runtime::block_on(async {
        let fixture = create_state_fixture(database.pool(), 3).await;
        let first = fixture.pages[0];
        let active = fixture.pages[1];
        let queued = fixture.pages[2];

        claim_render(database.pool(), first.page_id, first.attempt_id.unwrap())
            .await
            .unwrap();
        pause(database.pool(), fixture.run_id, IndexRunStatus::Running)
            .await
            .unwrap();
        let blocked_claim =
            claim_render(database.pool(), active.page_id, active.attempt_id.unwrap())
                .await
                .unwrap_err();
        assert_eq!(blocked_claim.code, AppErrorCode::RequestConflict);
        finish_from_rendering(database.pool(), first.page_id, first.attempt_id.unwrap()).await;

        resume(database.pool(), fixture.run_id, IndexRunStatus::Paused)
            .await
            .unwrap();
        claim_render(database.pool(), active.page_id, active.attempt_id.unwrap())
            .await
            .unwrap();
        advance_from_rendering_to_validating(
            database.pool(),
            active.page_id,
            active.attempt_id.unwrap(),
        )
        .await;
        let registry = IndexCancellationRegistry::default();
        let token = registry
            .register(fixture.run_id, active.page_id, active.attempt_id.unwrap())
            .unwrap();
        let cancelled = cancel_run(
            database.pool(),
            &registry,
            fixture.run_id,
            IndexRunStatus::Running,
        )
        .await
        .unwrap();
        assert_eq!(cancelled.active_attempts_signalled, 1);
        assert_eq!(cancelled.queued_pages_cancelled, 1);
        assert!(token.is_cancelled());
        assert_eq!(
            page_status(database.pool(), queued.page_id).await,
            IndexPageStatus::Cancelled
        );
        assert_eq!(
            page_status(database.pool(), active.page_id).await,
            IndexPageStatus::Validating
        );

        let late_commit = commit_indexed(
            database.pool(),
            active.page_id,
            active.attempt_id.unwrap(),
            &"d".repeat(64),
        )
        .await
        .unwrap_err();
        assert_eq!(late_commit.code, AppErrorCode::RequestConflict);

        cancel(
            database.pool(),
            active.page_id,
            IndexPageStatus::Validating,
            active.attempt_id.unwrap(),
        )
        .await
        .unwrap();
        assert!(registry.finish(fixture.run_id, active.page_id, active.attempt_id.unwrap()));
        assert_eq!(registry.active_count(), 0);
        assert_eq!(
            finalize_run_if_terminal(database.pool(), fixture.run_id, IndexRunStatus::Cancelling,)
                .await
                .unwrap(),
            IndexRunStatus::Cancelled
        );
        let aggregate = get_run_aggregate(database.pool(), fixture.run_id)
            .await
            .unwrap();
        assert_eq!(aggregate.control_status, IndexRunStatus::Cancelled);
        assert_eq!(aggregate.pages.indexed, 1);
        assert_eq!(aggregate.pages.cancelled, 2);
        assert_eq!(aggregate.aggregate_status, IndexAggregateStatus::Partial);
    });
}

struct StateFixture {
    run_id: Uuid,
    pages: Vec<IndexPageOwnership>,
}

async fn create_state_fixture(pool: &SqlitePool, page_count: u32) -> StateFixture {
    let timestamp = "2026-08-04T00:00:00.000Z";
    let book_id = Uuid::new_v4();
    let profile_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO books (id, sha256, title, format, original_filename, stored_path, import_status, created_at, updated_at) VALUES (?, ?, 'Synthetic state book', 'pdf', 'state.pdf', 'books/state/original.pdf', 'ready', ?, ?)",
    )
    .bind(book_id.to_string())
    .bind("a".repeat(64))
    .bind(timestamp)
    .bind(timestamp)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO provider_profiles (id, provider_kind, display_name, model_id, context_window_tokens, created_at, updated_at, validated_at) VALUES (?, 'openai', 'Synthetic state profile', 'synthetic-model', 32000, ?, ?, ?)",
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
    StateFixture { run_id, pages }
}

async fn drive_to_validating(pool: &SqlitePool, page_id: Uuid, attempt_id: Uuid) {
    claim_render(pool, page_id, attempt_id).await.unwrap();
    mark_rendered(pool, page_id, attempt_id, &"a".repeat(64))
        .await
        .unwrap();
    claim_send(pool, page_id, attempt_id).await.unwrap();
    mark_received(pool, page_id, attempt_id, &"b".repeat(64))
        .await
        .unwrap();
    claim_validate(pool, page_id, attempt_id).await.unwrap();
}

async fn finish_from_rendering(pool: &SqlitePool, page_id: Uuid, attempt_id: Uuid) {
    advance_from_rendering_to_validating(pool, page_id, attempt_id).await;
    commit_indexed(pool, page_id, attempt_id, &"c".repeat(64))
        .await
        .unwrap();
}

async fn advance_from_rendering_to_validating(pool: &SqlitePool, page_id: Uuid, attempt_id: Uuid) {
    mark_rendered(pool, page_id, attempt_id, &"a".repeat(64))
        .await
        .unwrap();
    claim_send(pool, page_id, attempt_id).await.unwrap();
    mark_received(pool, page_id, attempt_id, &"b".repeat(64))
        .await
        .unwrap();
    claim_validate(pool, page_id, attempt_id).await.unwrap();
}

async fn page_status(pool: &SqlitePool, page_id: Uuid) -> IndexPageStatus {
    let row = sqlx::query("SELECT status FROM index_pages WHERE id = ?")
        .bind(page_id.to_string())
        .fetch_one(pool)
        .await
        .unwrap();
    IndexPageStatus::from_database(&row.get::<String, _>("status")).unwrap()
}
