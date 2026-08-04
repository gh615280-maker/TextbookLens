use std::{collections::HashMap, sync::Arc};

use chrono::Utc;
use parking_lot::Mutex;
use sqlx::{Row, SqlitePool};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{
    db::indexing::{CreateIndexPage, IndexPageOwnership, create_index_page, database_timestamp},
    domain::{
        IndexFailureCode, IndexPageStatus, IndexQualityReason, IndexReviewReason, IndexRunStatus,
    },
    errors::{AppError, AppErrorCode, AppResult},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct ActiveAttemptKey {
    run_id: Uuid,
    page_id: Uuid,
    attempt_id: Uuid,
}

#[derive(Clone, Default)]
pub struct IndexCancellationRegistry {
    active: Arc<Mutex<HashMap<ActiveAttemptKey, CancellationToken>>>,
}

impl IndexCancellationRegistry {
    pub fn register(
        &self,
        run_id: Uuid,
        page_id: Uuid,
        attempt_id: Uuid,
    ) -> AppResult<CancellationToken> {
        let key = ActiveAttemptKey {
            run_id,
            page_id,
            attempt_id,
        };
        let mut active = self.active.lock();
        if active.keys().any(|candidate| candidate.page_id == page_id) {
            return Err(AppError::new(AppErrorCode::RequestConflict));
        }
        let token = CancellationToken::new();
        active.insert(key, token.clone());
        Ok(token)
    }

    pub fn finish(&self, run_id: Uuid, page_id: Uuid, attempt_id: Uuid) -> bool {
        self.active
            .lock()
            .remove(&ActiveAttemptKey {
                run_id,
                page_id,
                attempt_id,
            })
            .is_some()
    }

    pub fn cancel_run(&self, run_id: Uuid) -> usize {
        let active = self.active.lock();
        let mut cancelled = 0;
        for (key, token) in active.iter() {
            if key.run_id == run_id {
                token.cancel();
                cancelled += 1;
            }
        }
        cancelled
    }

    #[cfg(test)]
    pub(crate) fn active_count(&self) -> usize {
        self.active.lock().len()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClaimedPage {
    pub page_id: Uuid,
    pub run_id: Uuid,
    pub book_id: Uuid,
    pub page_number: u32,
    pub attempt_id: Uuid,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CancelRunResult {
    pub queued_pages_cancelled: u32,
    pub active_attempts_signalled: u32,
}

pub async fn queue(
    pool: &SqlitePool,
    run_id: Uuid,
    page_number: u32,
    quality_reason: IndexQualityReason,
    local_text_sha256: Option<String>,
) -> AppResult<IndexPageOwnership> {
    create_index_page(
        pool,
        CreateIndexPage {
            run_id,
            page_number,
            quality_reason,
            local_text_sha256,
        },
    )
    .await
}

pub async fn claim_render(
    pool: &SqlitePool,
    page_id: Uuid,
    attempt_id: Uuid,
) -> AppResult<ClaimedPage> {
    ensure_legal_transition(IndexPageStatus::Queued, IndexPageStatus::Rendering, false)?;
    let timestamp = database_timestamp(Utc::now());
    let result = sqlx::query(
        "UPDATE index_pages SET status = 'rendering', attempt_started_at = ?, updated_at = ? WHERE id = ? AND status = 'queued' AND attempt_id = ? AND EXISTS (SELECT 1 FROM index_runs r WHERE r.id = index_pages.run_id AND r.status = 'running')",
    )
    .bind(&timestamp)
    .bind(&timestamp)
    .bind(page_id.to_string())
    .bind(attempt_id.to_string())
    .execute(pool)
    .await?;
    require_one_row(result.rows_affected())?;
    claimed_page(pool, page_id, attempt_id, IndexPageStatus::Rendering).await
}

pub async fn mark_rendered(
    pool: &SqlitePool,
    page_id: Uuid,
    attempt_id: Uuid,
    render_sha256: &str,
) -> AppResult<()> {
    ensure_legal_transition(
        IndexPageStatus::Rendering,
        IndexPageStatus::Rendering,
        false,
    )?;
    validate_sha256(render_sha256)?;
    let timestamp = database_timestamp(Utc::now());
    let result = sqlx::query(
        "UPDATE index_pages SET render_sha256 = ?, updated_at = ? WHERE id = ? AND status = 'rendering' AND attempt_id = ?",
    )
    .bind(render_sha256)
    .bind(timestamp)
    .bind(page_id.to_string())
    .bind(attempt_id.to_string())
    .execute(pool)
    .await?;
    require_one_row(result.rows_affected())
}

pub async fn claim_send(pool: &SqlitePool, page_id: Uuid, attempt_id: Uuid) -> AppResult<()> {
    transition_page(
        pool,
        page_id,
        attempt_id,
        IndexPageStatus::Rendering,
        IndexPageStatus::Sending,
        "render_sha256 IS NOT NULL",
    )
    .await
}

pub async fn mark_received(
    pool: &SqlitePool,
    page_id: Uuid,
    attempt_id: Uuid,
    response_sha256: &str,
) -> AppResult<()> {
    ensure_legal_transition(IndexPageStatus::Sending, IndexPageStatus::Parsing, false)?;
    validate_sha256(response_sha256)?;
    let timestamp = database_timestamp(Utc::now());
    let result = sqlx::query(
        "UPDATE index_pages SET status = 'parsing', response_sha256 = ?, updated_at = ? WHERE id = ? AND status = 'sending' AND attempt_id = ?",
    )
    .bind(response_sha256)
    .bind(timestamp)
    .bind(page_id.to_string())
    .bind(attempt_id.to_string())
    .execute(pool)
    .await?;
    require_one_row(result.rows_affected())
}

pub async fn claim_validate(pool: &SqlitePool, page_id: Uuid, attempt_id: Uuid) -> AppResult<()> {
    transition_page(
        pool,
        page_id,
        attempt_id,
        IndexPageStatus::Parsing,
        IndexPageStatus::Validating,
        "1 = 1",
    )
    .await
}

pub async fn commit_review(
    pool: &SqlitePool,
    page_id: Uuid,
    attempt_id: Uuid,
    review_reason: IndexReviewReason,
    content_sha256: &str,
) -> AppResult<()> {
    ensure_legal_transition(
        IndexPageStatus::Validating,
        IndexPageStatus::NeedsReview,
        false,
    )?;
    validate_sha256(content_sha256)?;
    let timestamp = database_timestamp(Utc::now());
    let result = sqlx::query(
        "UPDATE index_pages SET status = 'needs_review', content_sha256 = ?, content_version = content_version + 1, review_reason_code = ?, retryable = 0, updated_at = ? WHERE id = ? AND status = 'validating' AND attempt_id = ? AND EXISTS (SELECT 1 FROM index_runs r WHERE r.id = index_pages.run_id AND r.status IN ('running', 'paused'))",
    )
    .bind(content_sha256)
    .bind(review_reason.as_str())
    .bind(timestamp)
    .bind(page_id.to_string())
    .bind(attempt_id.to_string())
    .execute(pool)
    .await?;
    require_one_row(result.rows_affected())
}

pub async fn commit_indexed(
    pool: &SqlitePool,
    page_id: Uuid,
    attempt_id: Uuid,
    content_sha256: &str,
) -> AppResult<()> {
    ensure_legal_transition(IndexPageStatus::Validating, IndexPageStatus::Indexed, false)?;
    validate_sha256(content_sha256)?;
    let timestamp = database_timestamp(Utc::now());
    let result = sqlx::query(
        "UPDATE index_pages SET status = 'indexed', content_sha256 = ?, content_version = content_version + 1, review_reason_code = NULL, retryable = 0, updated_at = ? WHERE id = ? AND status = 'validating' AND attempt_id = ? AND EXISTS (SELECT 1 FROM index_runs r WHERE r.id = index_pages.run_id AND r.status IN ('running', 'paused'))",
    )
    .bind(content_sha256)
    .bind(timestamp)
    .bind(page_id.to_string())
    .bind(attempt_id.to_string())
    .execute(pool)
    .await?;
    require_one_row(result.rows_affected())
}

pub async fn fail(
    pool: &SqlitePool,
    page_id: Uuid,
    expected_status: IndexPageStatus,
    attempt_id: Uuid,
    failure: IndexFailureCode,
    retryable: bool,
) -> AppResult<()> {
    ensure_legal_transition(expected_status, IndexPageStatus::Failed, false)?;
    let timestamp = database_timestamp(Utc::now());
    let result = sqlx::query(
        "UPDATE index_pages SET status = 'failed', review_reason_code = NULL, safe_error_code = ?, safe_error_message = ?, retryable = ?, updated_at = ? WHERE id = ? AND status = ? AND attempt_id = ?",
    )
    .bind(failure.as_str())
    .bind(failure.safe_message())
    .bind(i64::from(retryable))
    .bind(timestamp)
    .bind(page_id.to_string())
    .bind(expected_status.as_str())
    .bind(attempt_id.to_string())
    .execute(pool)
    .await?;
    require_one_row(result.rows_affected())
}

pub async fn cancel(
    pool: &SqlitePool,
    page_id: Uuid,
    expected_status: IndexPageStatus,
    attempt_id: Uuid,
) -> AppResult<()> {
    ensure_legal_transition(expected_status, IndexPageStatus::Cancelled, false)?;
    let timestamp = database_timestamp(Utc::now());
    let result = sqlx::query(
        "UPDATE index_pages SET status = 'cancelled', review_reason_code = NULL, safe_error_code = NULL, safe_error_message = NULL, retryable = 0, updated_at = ? WHERE id = ? AND status = ? AND attempt_id = ?",
    )
    .bind(timestamp)
    .bind(page_id.to_string())
    .bind(expected_status.as_str())
    .bind(attempt_id.to_string())
    .execute(pool)
    .await?;
    require_one_row(result.rows_affected())
}

pub async fn retry(
    pool: &SqlitePool,
    page_id: Uuid,
    expected_status: IndexPageStatus,
    expected_attempt_id: Uuid,
) -> AppResult<Uuid> {
    ensure_legal_transition(expected_status, IndexPageStatus::Queued, true)?;
    let new_attempt_id = Uuid::new_v4();
    let timestamp = database_timestamp(Utc::now());
    let result = sqlx::query(
        "UPDATE index_pages SET status = 'queued', attempt_id = ?, attempt_count = attempt_count + 1, attempt_started_at = NULL, render_sha256 = NULL, response_sha256 = NULL, review_reason_code = NULL, safe_error_code = NULL, safe_error_message = NULL, retryable = 0, updated_at = ? WHERE id = ? AND status = ? AND attempt_id = ?",
    )
    .bind(new_attempt_id.to_string())
    .bind(timestamp)
    .bind(page_id.to_string())
    .bind(expected_status.as_str())
    .bind(expected_attempt_id.to_string())
    .execute(pool)
    .await?;
    require_one_row(result.rows_affected())?;
    Ok(new_attempt_id)
}

pub async fn pause(
    pool: &SqlitePool,
    run_id: Uuid,
    expected_status: IndexRunStatus,
) -> AppResult<()> {
    if expected_status != IndexRunStatus::Running {
        return Err(AppError::new(AppErrorCode::RequestConflict));
    }
    let timestamp = database_timestamp(Utc::now());
    let result = sqlx::query(
        "UPDATE index_runs SET status = 'paused', paused_at = ?, updated_at = ? WHERE id = ? AND status = ?",
    )
    .bind(&timestamp)
    .bind(&timestamp)
    .bind(run_id.to_string())
    .bind(expected_status.as_str())
    .execute(pool)
    .await?;
    require_one_row(result.rows_affected())
}

pub async fn resume(
    pool: &SqlitePool,
    run_id: Uuid,
    expected_status: IndexRunStatus,
) -> AppResult<()> {
    if expected_status != IndexRunStatus::Paused {
        return Err(AppError::new(AppErrorCode::RequestConflict));
    }
    let timestamp = database_timestamp(Utc::now());
    let result = sqlx::query(
        "UPDATE index_runs SET status = 'running', paused_at = NULL, updated_at = ? WHERE id = ? AND status = ?",
    )
    .bind(timestamp)
    .bind(run_id.to_string())
    .bind(expected_status.as_str())
    .execute(pool)
    .await?;
    require_one_row(result.rows_affected())
}

pub async fn cancel_run(
    pool: &SqlitePool,
    cancellations: &IndexCancellationRegistry,
    run_id: Uuid,
    expected_status: IndexRunStatus,
) -> AppResult<CancelRunResult> {
    if !matches!(
        expected_status,
        IndexRunStatus::Running | IndexRunStatus::Paused
    ) {
        return Err(AppError::new(AppErrorCode::RequestConflict));
    }
    let timestamp = database_timestamp(Utc::now());
    let mut transaction = pool.begin().await?;
    let run_update = sqlx::query(
        "UPDATE index_runs SET status = 'cancelling', paused_at = NULL, cancel_requested_at = ?, updated_at = ? WHERE id = ? AND status = ?",
    )
    .bind(&timestamp)
    .bind(&timestamp)
    .bind(run_id.to_string())
    .bind(expected_status.as_str())
    .execute(&mut *transaction)
    .await?;
    require_one_row(run_update.rows_affected())?;

    let queued = sqlx::query(
        "SELECT id, attempt_id FROM index_pages WHERE run_id = ? AND status = 'queued' ORDER BY page_number",
    )
    .bind(run_id.to_string())
    .fetch_all(&mut *transaction)
    .await?;
    let mut queued_pages_cancelled = 0_u32;
    for row in queued {
        let page_id: String = row.try_get("id")?;
        let attempt_id: String = row.try_get("attempt_id")?;
        let page_update = sqlx::query(
            "UPDATE index_pages SET status = 'cancelled', retryable = 0, updated_at = ? WHERE id = ? AND status = 'queued' AND attempt_id = ?",
        )
        .bind(&timestamp)
        .bind(page_id)
        .bind(attempt_id)
        .execute(&mut *transaction)
        .await?;
        require_one_row(page_update.rows_affected())?;
        queued_pages_cancelled = queued_pages_cancelled
            .checked_add(1)
            .ok_or_else(database_error)?;
    }
    transaction.commit().await?;

    let active_attempts_signalled =
        u32::try_from(cancellations.cancel_run(run_id)).map_err(|_| database_error())?;
    Ok(CancelRunResult {
        queued_pages_cancelled,
        active_attempts_signalled,
    })
}

pub async fn finalize_run_if_terminal(
    pool: &SqlitePool,
    run_id: Uuid,
    expected_status: IndexRunStatus,
) -> AppResult<IndexRunStatus> {
    let target = match expected_status {
        IndexRunStatus::Running | IndexRunStatus::Paused => IndexRunStatus::Completed,
        IndexRunStatus::Cancelling => IndexRunStatus::Cancelled,
        IndexRunStatus::Cancelled | IndexRunStatus::Completed => {
            return Err(AppError::new(AppErrorCode::RequestConflict));
        }
    };
    let timestamp = database_timestamp(Utc::now());
    let result = if target == IndexRunStatus::Cancelled {
        sqlx::query(
            "UPDATE index_runs SET status = 'cancelled', paused_at = NULL, completed_at = ?, updated_at = ? WHERE id = ? AND status = ? AND NOT EXISTS (SELECT 1 FROM index_pages WHERE run_id = ? AND status IN ('queued', 'rendering', 'sending', 'parsing', 'validating'))",
        )
        .bind(&timestamp)
        .bind(&timestamp)
        .bind(run_id.to_string())
        .bind(expected_status.as_str())
        .bind(run_id.to_string())
        .execute(pool)
        .await?
    } else {
        sqlx::query(
            "UPDATE index_runs SET status = 'completed', paused_at = NULL, completed_at = ?, updated_at = ? WHERE id = ? AND status = ? AND NOT EXISTS (SELECT 1 FROM index_pages WHERE run_id = ? AND status IN ('queued', 'rendering', 'sending', 'parsing', 'validating'))",
        )
        .bind(&timestamp)
        .bind(&timestamp)
        .bind(run_id.to_string())
        .bind(expected_status.as_str())
        .bind(run_id.to_string())
        .execute(pool)
        .await?
    };
    require_one_row(result.rows_affected())?;
    Ok(target)
}

async fn transition_page(
    pool: &SqlitePool,
    page_id: Uuid,
    attempt_id: Uuid,
    expected_status: IndexPageStatus,
    target_status: IndexPageStatus,
    additional_predicate: &'static str,
) -> AppResult<()> {
    ensure_legal_transition(expected_status, target_status, false)?;
    let timestamp = database_timestamp(Utc::now());
    let query = match additional_predicate {
        "render_sha256 IS NOT NULL" => sqlx::query(
            "UPDATE index_pages SET status = ?, updated_at = ? WHERE id = ? AND status = ? AND attempt_id = ? AND render_sha256 IS NOT NULL",
        ),
        "1 = 1" => sqlx::query(
            "UPDATE index_pages SET status = ?, updated_at = ? WHERE id = ? AND status = ? AND attempt_id = ?",
        ),
        _ => return Err(database_error()),
    };
    let result = query
        .bind(target_status.as_str())
        .bind(timestamp)
        .bind(page_id.to_string())
        .bind(expected_status.as_str())
        .bind(attempt_id.to_string())
        .execute(pool)
        .await?;
    require_one_row(result.rows_affected())
}

async fn claimed_page(
    pool: &SqlitePool,
    page_id: Uuid,
    attempt_id: Uuid,
    expected_status: IndexPageStatus,
) -> AppResult<ClaimedPage> {
    let row = sqlx::query(
        "SELECT run_id, book_id, page_number FROM index_pages WHERE id = ? AND status = ? AND attempt_id = ?",
    )
    .bind(page_id.to_string())
    .bind(expected_status.as_str())
    .bind(attempt_id.to_string())
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::new(AppErrorCode::RequestConflict))?;
    Ok(ClaimedPage {
        page_id,
        run_id: parse_uuid(&row.try_get::<String, _>("run_id")?)?,
        book_id: parse_uuid(&row.try_get::<String, _>("book_id")?)?,
        page_number: u32::try_from(row.try_get::<i64, _>("page_number")?)
            .map_err(|_| database_error())?,
        attempt_id,
    })
}

pub(crate) fn ensure_legal_transition(
    from: IndexPageStatus,
    to: IndexPageStatus,
    explicit_retry: bool,
) -> AppResult<()> {
    if legal_transition(from, to, explicit_retry) {
        Ok(())
    } else {
        Err(AppError::new(AppErrorCode::RequestConflict))
    }
}

pub(crate) const fn legal_transition(
    from: IndexPageStatus,
    to: IndexPageStatus,
    explicit_retry: bool,
) -> bool {
    if explicit_retry {
        return matches!(
            (from, to),
            (
                IndexPageStatus::Indexed
                    | IndexPageStatus::NeedsReview
                    | IndexPageStatus::Failed
                    | IndexPageStatus::Cancelled,
                IndexPageStatus::Queued
            )
        );
    }
    matches!(
        (from, to),
        (IndexPageStatus::Queued, IndexPageStatus::Rendering)
            | (IndexPageStatus::Queued, IndexPageStatus::Failed)
            | (IndexPageStatus::Queued, IndexPageStatus::Cancelled)
            | (IndexPageStatus::Rendering, IndexPageStatus::Rendering)
            | (IndexPageStatus::Rendering, IndexPageStatus::Sending)
            | (IndexPageStatus::Rendering, IndexPageStatus::Failed)
            | (IndexPageStatus::Rendering, IndexPageStatus::Cancelled)
            | (IndexPageStatus::Sending, IndexPageStatus::Parsing)
            | (IndexPageStatus::Sending, IndexPageStatus::Failed)
            | (IndexPageStatus::Sending, IndexPageStatus::Cancelled)
            | (IndexPageStatus::Parsing, IndexPageStatus::Validating)
            | (IndexPageStatus::Parsing, IndexPageStatus::Failed)
            | (IndexPageStatus::Parsing, IndexPageStatus::Cancelled)
            | (IndexPageStatus::Validating, IndexPageStatus::Indexed)
            | (IndexPageStatus::Validating, IndexPageStatus::NeedsReview)
            | (IndexPageStatus::Validating, IndexPageStatus::Failed)
            | (IndexPageStatus::Validating, IndexPageStatus::Cancelled)
    )
}

fn validate_sha256(value: &str) -> AppResult<()> {
    if value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(())
    } else {
        Err(AppError::new(AppErrorCode::InvalidInput))
    }
}

fn parse_uuid(value: &str) -> AppResult<Uuid> {
    Uuid::parse_str(value).map_err(|_| database_error())
}

fn require_one_row(rows_affected: u64) -> AppResult<()> {
    match rows_affected {
        1 => Ok(()),
        0 => Err(AppError::new(AppErrorCode::RequestConflict)),
        _ => Err(database_error()),
    }
}

fn database_error() -> AppError {
    AppError::new(AppErrorCode::DatabaseError)
}
