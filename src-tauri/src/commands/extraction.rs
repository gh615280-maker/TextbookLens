use tauri::State;
use uuid::Uuid;

use crate::{
    app_state::AppState,
    errors::AppErrorDto,
    extraction::{
        kimi_files::KimiFilesClient,
        service::{self, ExtractionSummary},
    },
};

#[tauri::command]
pub async fn prepare_book_extraction(
    state: State<'_, AppState>,
    book_id: Uuid,
    provider_profile_id: Uuid,
) -> Result<ExtractionSummary, AppErrorDto> {
    let cancel = state.extraction_cancellations.begin(book_id)?;
    let client = KimiFilesClient::new()?;
    let result = service::prepare(
        state.db.pool(),
        &state.paths,
        state.credential_store.clone(),
        state.provider_capabilities.clone(),
        &client,
        book_id,
        provider_profile_id,
        cancel,
    )
    .await;
    state.extraction_cancellations.finish(book_id);
    result.map_err(Into::into)
}

#[tauri::command]
pub async fn cancel_book_extraction(
    state: State<'_, AppState>,
    book_id: Uuid,
) -> Result<(), AppErrorDto> {
    state.extraction_cancellations.cancel(book_id);
    Ok(())
}

#[tauri::command]
pub async fn get_book_extraction(
    state: State<'_, AppState>,
    book_id: Uuid,
) -> Result<Option<ExtractionSummary>, AppErrorDto> {
    let id: Option<String> = sqlx::query_scalar(
        "SELECT id FROM book_extractions WHERE book_id = ? ORDER BY updated_at DESC LIMIT 1",
    )
    .bind(book_id.to_string())
    .fetch_optional(state.db.pool())
    .await
    .map_err(crate::errors::AppError::database)?;
    match id {
        Some(id) => Ok(Some(
            service::summary(
                state.db.pool(),
                Uuid::parse_str(&id).map_err(|_| {
                    crate::errors::AppError::new(crate::errors::AppErrorCode::DatabaseError)
                })?,
                false,
            )
            .await?,
        )),
        None => Ok(None),
    }
}
