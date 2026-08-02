use std::fs;

use sqlx::SqlitePool;
use uuid::Uuid;

use crate::{
    app_state::AppPaths,
    book_repository as books,
    domain::ImportStatus,
    errors::{AppError, AppErrorCode, AppResult},
};

use super::storage::resolve_owned_source;

pub async fn read_book_source_bytes(
    pool: &SqlitePool,
    paths: &AppPaths,
    book_id: Uuid,
) -> AppResult<Vec<u8>> {
    let record = books::get(pool, book_id).await?;
    if record.summary.import_status != ImportStatus::Parsing
        && record.summary.import_status != ImportStatus::Ready
    {
        return Err(AppError::new(AppErrorCode::BookNotReady));
    }
    let stored_path = record
        .stored_path
        .as_deref()
        .ok_or_else(|| AppError::new(AppErrorCode::BookNotReady))?;
    let owned_path = resolve_owned_source(paths, book_id, &record.summary.format, stored_path)?;
    fs::read(owned_path).map_err(AppError::from)
}
