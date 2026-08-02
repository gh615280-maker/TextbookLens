use std::fs;

use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::{
    app_state::AppPaths,
    errors::{AppError, AppErrorCode, AppResult},
};

use super::storage::resolve_owned_source;

pub async fn read_book_source_bytes(
    pool: &SqlitePool,
    paths: &AppPaths,
    book_id: Uuid,
) -> AppResult<Vec<u8>> {
    let row = sqlx::query("SELECT import_status, stored_path FROM books WHERE id = ?")
        .bind(book_id.to_string())
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| AppError::new(AppErrorCode::NotFound))?;
    let status: String = row.try_get("import_status")?;
    if status != "parsing" && status != "ready" {
        return Err(AppError::new(AppErrorCode::BookNotReady));
    }
    let stored_path: String = row.try_get("stored_path")?;
    let owned_path = resolve_owned_source(paths, book_id, &stored_path)?;
    fs::read(owned_path).map_err(AppError::from)
}
