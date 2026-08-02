use tauri::State;
use uuid::Uuid;

use crate::{
    app_state::AppState, documents::import::ImportService, domain::BookSummary, errors::AppErrorDto,
};

fn service(state: &AppState) -> ImportService {
    ImportService::with_cancellations(
        state.db.pool().clone(),
        state.paths.clone(),
        state.import_cancellations.clone(),
    )
}

#[tauri::command]
pub async fn list_books(state: State<'_, AppState>) -> Result<Vec<BookSummary>, AppErrorDto> {
    service(&state)
        .list_books()
        .await
        .map_err(AppErrorDto::from)
}

#[tauri::command]
pub async fn get_book(
    state: State<'_, AppState>,
    book_id: Uuid,
) -> Result<BookSummary, AppErrorDto> {
    service(&state)
        .get_book(book_id)
        .await
        .map_err(AppErrorDto::from)
}

#[tauri::command]
pub async fn delete_failed_import(
    state: State<'_, AppState>,
    book_id: Uuid,
) -> Result<(), AppErrorDto> {
    service(&state)
        .delete_failed_import(book_id)
        .await
        .map_err(AppErrorDto::from)
}

#[tauri::command]
pub async fn list_reader_sections(
    state: State<'_, AppState>,
    book_id: Uuid,
) -> Result<Vec<crate::book_repository::ReaderSection>, AppErrorDto> {
    crate::book_repository::list_reader_sections(state.db.pool(), book_id)
        .await
        .map_err(AppErrorDto::from)
}
