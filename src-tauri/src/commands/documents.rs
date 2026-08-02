use std::sync::Arc;

use tauri::{
    State,
    ipc::{Channel, Response},
};
use uuid::Uuid;

use crate::{
    app_state::AppState,
    documents::{
        import::{
            BeginImportOutcome, BeginImportRequest, ImportEvent, ImportService, ImportStage,
            ParsedBookMetadata,
        },
        storage::noop_progress,
    },
    domain::{BookSummary, NormalizedSectionInput},
    errors::{AppErrorCode, AppErrorDto},
};

fn service(state: &AppState) -> ImportService {
    ImportService::with_cancellations(
        state.db.pool().clone(),
        state.paths.clone(),
        state.import_cancellations.clone(),
    )
}

fn channel_progress(progress: Channel<ImportEvent>) -> Arc<dyn Fn(ImportEvent) + Send + Sync> {
    Arc::new(move |event| {
        let _ = progress.send(event);
    })
}

#[tauri::command]
pub async fn begin_import(
    state: State<'_, AppState>,
    request: BeginImportRequest,
    progress: Channel<ImportEvent>,
) -> Result<BeginImportOutcome, AppErrorDto> {
    service(&state)
        .begin_import(request, channel_progress(progress))
        .await
        .map_err(AppErrorDto::from)
}

#[tauri::command]
pub async fn read_book_source(
    state: State<'_, AppState>,
    book_id: Uuid,
) -> Result<Response, AppErrorDto> {
    service(&state)
        .read_book_source(book_id)
        .await
        .map(Response::new)
        .map_err(AppErrorDto::from)
}

#[tauri::command]
pub async fn begin_parse(
    state: State<'_, AppState>,
    book_id: Uuid,
    metadata: ParsedBookMetadata,
) -> Result<(), AppErrorDto> {
    service(&state)
        .begin_parse(book_id, metadata)
        .await
        .map_err(AppErrorDto::from)
}

#[tauri::command]
pub async fn append_parsed_sections(
    state: State<'_, AppState>,
    book_id: Uuid,
    sections: Vec<NormalizedSectionInput>,
) -> Result<(), AppErrorDto> {
    service(&state)
        .append_parsed_sections(book_id, sections)
        .await
        .map_err(AppErrorDto::from)
}

#[tauri::command]
pub async fn finalize_import(
    state: State<'_, AppState>,
    book_id: Uuid,
    progress: Channel<ImportEvent>,
) -> Result<BookSummary, AppErrorDto> {
    service(&state)
        .finalize_import(book_id, channel_progress(progress))
        .await
        .map_err(AppErrorDto::from)
}

#[tauri::command]
pub async fn cancel_import(state: State<'_, AppState>, book_id: Uuid) -> Result<(), AppErrorDto> {
    service(&state)
        .cancel_import(book_id)
        .await
        .map_err(AppErrorDto::from)
}

#[tauri::command]
pub async fn mark_import_failed(
    state: State<'_, AppState>,
    book_id: Uuid,
    stage: ImportStage,
    code: AppErrorCode,
) -> Result<(), AppErrorDto> {
    service(&state)
        .mark_import_failed(book_id, stage, code)
        .await
        .map_err(AppErrorDto::from)
}

#[tauri::command]
pub async fn retry_import(
    state: State<'_, AppState>,
    book_id: Uuid,
    replacement_source_path: Option<String>,
) -> Result<BeginImportOutcome, AppErrorDto> {
    service(&state)
        .retry_import(book_id, replacement_source_path, noop_progress())
        .await
        .map_err(AppErrorDto::from)
}
