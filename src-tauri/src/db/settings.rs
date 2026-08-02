use std::collections::HashSet;

use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::{
    app_state::AppPaths,
    documents::storage::{recover_import_storage, remove_book_directory},
    errors::{AppError, AppErrorCode, AppResult},
};

pub fn recover_interrupted_imports(pool: &SqlitePool, paths: &AppPaths) -> AppResult<()> {
    let rows = tauri::async_runtime::block_on(async {
        sqlx::query("SELECT id, import_status FROM books")
            .fetch_all(pool)
            .await
            .map_err(AppError::from)
    })?;
    let mut owned_book_ids = HashSet::with_capacity(rows.len());
    let mut incomplete_copy_ids = Vec::new();
    for row in rows {
        let id = Uuid::parse_str(&row.try_get::<String, _>("id")?)
            .map_err(|_| AppError::new(AppErrorCode::DatabaseError))?;
        let status: String = row.try_get("import_status")?;
        owned_book_ids.insert(id);
        if status == "queued" || status == "copying" {
            incomplete_copy_ids.push(id);
        }
    }

    recover_import_storage(paths, &owned_book_ids)?;
    for id in incomplete_copy_ids {
        remove_book_directory(paths, id)?;
    }

    tauri::async_runtime::block_on(async {
        sqlx::query(
            "UPDATE books SET import_status = 'failed', import_error_code = ?, import_error_message = ?, import_error_stage = CASE WHEN import_status IN ('queued', 'copying') THEN 'copying' ELSE import_status END, sha256 = CASE WHEN import_status IN ('queued', 'copying') THEN NULL ELSE sha256 END, stored_path = CASE WHEN import_status IN ('queued', 'copying') THEN NULL ELSE stored_path END, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE import_status IN ('queued', 'copying', 'parsing', 'indexing')",
        )
        .bind("IMPORT_CANCELLED")
        .bind("上次导入因应用退出而中断，请重试")
        .execute(pool)
        .await
        .map_err(AppError::from)?;
        Ok(())
    })
}
