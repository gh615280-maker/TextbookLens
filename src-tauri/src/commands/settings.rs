use serde::Serialize;
use tauri::State;
use uuid::Uuid;

use crate::{
    app_state::AppState,
    db::settings,
    domain::{BookSummary, DocumentLocator, ReaderSettingsDto},
    errors::{AppError, AppErrorCode, AppErrorDto},
};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReaderBootstrap {
    pub book: BookSummary,
    pub last_locator: Option<DocumentLocator>,
}

#[tauri::command]
pub async fn get_reader_bootstrap(
    state: State<'_, AppState>,
    book_id: Uuid,
) -> Result<ReaderBootstrap, AppErrorDto> {
    let book = crate::book_repository::get(state.db.pool(), book_id)
        .await
        .map_err(AppErrorDto::from)?
        .summary;
    let locator_json: Option<String> =
        sqlx::query_scalar("SELECT last_locator_json FROM books WHERE id = ?")
            .bind(book_id.to_string())
            .fetch_one(state.db.pool())
            .await
            .map_err(AppError::from)
            .map_err(AppErrorDto::from)?;
    let last_locator = locator_json
        .map(|json| serde_json::from_str(&json).map_err(AppError::database))
        .transpose()
        .map_err(AppErrorDto::from)?;
    Ok(ReaderBootstrap { book, last_locator })
}

#[tauri::command]
pub async fn save_reading_progress(
    state: State<'_, AppState>,
    book_id: Uuid,
    progress: f64,
    locator: DocumentLocator,
) -> Result<(), AppErrorDto> {
    let book = crate::book_repository::get(state.db.pool(), book_id)
        .await
        .map_err(AppErrorDto::from)?;
    if !locator_matches_book(&locator, &book.summary.format) {
        return Err(AppError::new(AppErrorCode::InvalidInput).into());
    }
    settings::save_reading_progress(state.db.pool(), book_id, progress, &locator)
        .await
        .map_err(AppErrorDto::from)
}

#[tauri::command]
pub async fn get_reader_settings(
    state: State<'_, AppState>,
) -> Result<ReaderSettingsDto, AppErrorDto> {
    settings::get_reader_settings(state.db.pool())
        .await
        .map_err(AppErrorDto::from)
}

#[tauri::command]
pub async fn update_reader_settings(
    state: State<'_, AppState>,
    settings: ReaderSettingsDto,
) -> Result<ReaderSettingsDto, AppErrorDto> {
    settings::update_reader_settings(state.db.pool(), settings)
        .await
        .map_err(AppErrorDto::from)
}

fn locator_matches_book(locator: &DocumentLocator, format: &crate::domain::BookFormat) -> bool {
    matches!(
        (locator, format),
        (DocumentLocator::Pdf { .. }, crate::domain::BookFormat::Pdf)
            | (
                DocumentLocator::Epub { .. },
                crate::domain::BookFormat::Epub
            )
            | (
                DocumentLocator::Docx { .. },
                crate::domain::BookFormat::Docx
            )
    )
}
