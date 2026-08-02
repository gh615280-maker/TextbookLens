use std::{fs, sync::Arc};

use serde::Deserialize;
use tauri::{
    State,
    ipc::{Channel, Response},
};
use uuid::Uuid;

use crate::{
    app_state::AppState,
    documents::{
        derived::{remove_document_html, write_document_html},
        import::{
            BeginImportOutcome, BeginImportRequest, ImportEvent, ImportService, ImportStage,
            ParsedBookMetadata,
        },
        storage::noop_progress,
    },
    domain::{BookFormat, BookSummary, NormalizedSectionInput},
    errors::{AppError, AppErrorCode, AppErrorDto},
    retrieval::search::{SearchHit, search_book as search_book_repository},
};

#[derive(Clone, Copy, Debug, Deserialize)]
pub enum DerivedTextName {
    #[serde(rename = "document.html")]
    DocumentHtml,
}

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
    if metadata.title.trim().is_empty() {
        return Err(AppError::new(AppErrorCode::InvalidInput).into());
    }
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
pub async fn write_derived_text(
    state: State<'_, AppState>,
    book_id: Uuid,
    name: DerivedTextName,
    content: String,
) -> Result<(), AppErrorDto> {
    if content.is_empty() {
        return Err(AppError::new(AppErrorCode::InvalidInput).into());
    }
    ensure_import_not_cancelled(&state, book_id)?;
    crate::book_repository::require_parsing(state.db.pool(), book_id)
        .await
        .map_err(AppErrorDto::from)?;
    if crate::book_repository::get(state.db.pool(), book_id)
        .await
        .map_err(AppErrorDto::from)?
        .summary
        .format
        != BookFormat::Docx
    {
        return Err(AppError::new(AppErrorCode::InvalidInput).into());
    }

    let DerivedTextName::DocumentHtml = name;
    let book_directory = state.paths.books.join(book_id.to_string());
    let paths = state.paths.clone();
    let write_result =
        tokio::task::spawn_blocking(move || write_document_html(&paths, book_id, &content))
            .await
            .map_err(|_| AppErrorDto::from(AppError::new(AppErrorCode::LocalIoError)))?;
    write_result.map_err(AppErrorDto::from)?;

    if let Err(error) = ensure_import_not_cancelled(&state, book_id) {
        let _ = service(&state).cancel_import(book_id).await;
        return Err(error);
    }
    if let Err(error) = crate::book_repository::require_parsing(state.db.pool(), book_id).await {
        let cancelled = crate::book_repository::get(state.db.pool(), book_id)
            .await
            .ok()
            .and_then(|record| record.summary.import_error_code)
            .is_some_and(|code| code == "IMPORT_CANCELLED");
        if cancelled {
            let _ = fs::remove_dir_all(&book_directory);
        } else {
            let _ = remove_document_html(&state.paths, book_id);
        }
        return Err(error.into());
    }
    Ok(())
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

#[tauri::command]
pub async fn search_book(
    state: State<'_, AppState>,
    book_id: Uuid,
    query: String,
    limit: u32,
) -> Result<Vec<SearchHit>, AppErrorDto> {
    search_book_repository(state.db.pool(), book_id, &query, limit)
        .await
        .map_err(AppErrorDto::from)
}

fn ensure_import_not_cancelled(state: &AppState, book_id: Uuid) -> Result<(), AppErrorDto> {
    if state.import_cancellations.is_cancelled(book_id) {
        Err(AppError::new(AppErrorCode::ImportCancelled).into())
    } else {
        Ok(())
    }
}
