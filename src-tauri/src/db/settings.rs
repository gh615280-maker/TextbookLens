use sqlx::SqlitePool;

use crate::errors::{AppError, AppResult};

pub fn recover_interrupted_imports(pool: &SqlitePool) -> AppResult<()> {
    tauri::async_runtime::block_on(async {
        sqlx::query(
            "UPDATE books SET import_status = 'failed', import_error_code = ?, import_error_message = ?, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE import_status IN ('copying', 'parsing', 'indexing')",
        )
        .bind("IMPORT_CANCELLED")
        .bind("上次导入被应用退出中断，请重试")
        .execute(pool)
        .await
        .map_err(AppError::from)?;
        Ok(())
    })
}
