use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use chrono::{DateTime, Utc};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::{
    db::indexing::{database_timestamp, parse_page_status, parse_timestamp},
    domain::{IndexFailureCode, IndexPageStatus},
    errors::{AppError, AppErrorCode, AppResult},
};

pub const DEFAULT_RECOVERY_MAX_AGE: Duration = Duration::from_secs(15 * 60);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RecoverySummary {
    pub requeued: u32,
    pub marked_retryable: u32,
    pub unchanged_indexed: u32,
}

struct InterruptedPage {
    page_id: Uuid,
    status: IndexPageStatus,
    attempt_id: Uuid,
    updated_at: DateTime<Utc>,
    has_render_hash: bool,
}

pub fn scratch_path(root: &Path, page_id: Uuid, attempt_id: Uuid) -> PathBuf {
    root.join(page_id.to_string())
        .join(format!("{attempt_id}.render"))
}

pub fn recover_on_startup(pool: &SqlitePool, scratch_root: &Path) -> AppResult<RecoverySummary> {
    fs::create_dir_all(scratch_root)?;
    tauri::async_runtime::block_on(recover_interrupted_pages(
        pool,
        scratch_root,
        Utc::now(),
        DEFAULT_RECOVERY_MAX_AGE,
    ))
}

pub async fn recover_interrupted_pages(
    pool: &SqlitePool,
    scratch_root: &Path,
    now: DateTime<Utc>,
    max_attempt_age: Duration,
) -> AppResult<RecoverySummary> {
    fs::create_dir_all(scratch_root)?;
    let max_age = chrono::Duration::from_std(max_attempt_age)
        .map_err(|_| AppError::new(AppErrorCode::InvalidInput))?;
    let rows = sqlx::query(
        "SELECT id, status, attempt_id, updated_at, render_sha256 FROM index_pages WHERE status IN ('rendering', 'sending', 'parsing', 'validating') ORDER BY run_id, page_number",
    )
    .fetch_all(pool)
    .await?;
    let interrupted = rows
        .into_iter()
        .map(|row| {
            let attempt_id = row
                .try_get::<Option<String>, _>("attempt_id")?
                .ok_or_else(database_error)?;
            Ok(InterruptedPage {
                page_id: parse_uuid(&row.try_get::<String, _>("id")?)?,
                status: parse_page_status(&row.try_get::<String, _>("status")?)?,
                attempt_id: parse_uuid(&attempt_id)?,
                updated_at: parse_timestamp(&row.try_get::<String, _>("updated_at")?)?,
                has_render_hash: row.try_get::<Option<String>, _>("render_sha256")?.is_some(),
            })
        })
        .collect::<AppResult<Vec<_>>>()?;

    let unchanged_indexed = u32::try_from(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM index_pages WHERE status = 'indexed'")
            .fetch_one(pool)
            .await?,
    )
    .map_err(|_| database_error())?;
    let mut summary = RecoverySummary {
        unchanged_indexed,
        ..RecoverySummary::default()
    };

    for page in interrupted {
        let age = now.signed_duration_since(page.updated_at);
        let fresh = age <= max_age;
        let old_scratch = scratch_path(scratch_root, page.page_id, page.attempt_id);
        let scratch_exists = old_scratch.is_file();
        let can_reuse_scratch = scratch_exists && fresh && page.has_render_hash;
        let should_requeue = page.status == IndexPageStatus::Rendering || can_reuse_scratch;

        if should_requeue {
            let new_attempt_id = Uuid::new_v4();
            let new_scratch = scratch_path(scratch_root, page.page_id, new_attempt_id);
            if can_reuse_scratch {
                move_scratch(&old_scratch, &new_scratch)?;
            } else if scratch_exists {
                fs::remove_file(&old_scratch)?;
            }

            let timestamp = database_timestamp(now);
            let result = sqlx::query(
                "UPDATE index_pages SET status = 'queued', attempt_id = ?, attempt_count = attempt_count + 1, attempt_started_at = NULL, render_sha256 = CASE WHEN ? = 1 THEN render_sha256 ELSE NULL END, response_sha256 = NULL, review_reason_code = NULL, safe_error_code = NULL, safe_error_message = NULL, retryable = 0, updated_at = ? WHERE id = ? AND status = ? AND attempt_id = ?",
            )
            .bind(new_attempt_id.to_string())
            .bind(i64::from(can_reuse_scratch))
            .bind(&timestamp)
            .bind(page.page_id.to_string())
            .bind(page.status.as_str())
            .bind(page.attempt_id.to_string())
            .execute(pool)
            .await?;
            if let Err(error) = require_one_row(result.rows_affected()) {
                if can_reuse_scratch {
                    let _ = move_scratch(&new_scratch, &old_scratch);
                }
                return Err(error);
            }
            summary.requeued = summary.requeued.checked_add(1).ok_or_else(database_error)?;
            continue;
        }

        let failure = if scratch_exists {
            fs::remove_file(&old_scratch)?;
            IndexFailureCode::IndexAttemptExpired
        } else {
            IndexFailureCode::IndexScratchMissing
        };
        let timestamp = database_timestamp(now);
        let result = sqlx::query(
            "UPDATE index_pages SET status = 'failed', review_reason_code = NULL, safe_error_code = ?, safe_error_message = ?, retryable = 1, updated_at = ? WHERE id = ? AND status = ? AND attempt_id = ?",
        )
        .bind(failure.as_str())
        .bind(failure.safe_message())
        .bind(timestamp)
        .bind(page.page_id.to_string())
        .bind(page.status.as_str())
        .bind(page.attempt_id.to_string())
        .execute(pool)
        .await?;
        require_one_row(result.rows_affected())?;
        summary.marked_retryable = summary
            .marked_retryable
            .checked_add(1)
            .ok_or_else(database_error)?;
    }

    Ok(summary)
}

fn move_scratch(source: &Path, target: &Path) -> AppResult<()> {
    let parent = target
        .parent()
        .ok_or_else(|| AppError::new(AppErrorCode::LocalIoError))?;
    fs::create_dir_all(parent)?;
    fs::rename(source, target)?;
    Ok(())
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
