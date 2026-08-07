use tauri::State;
use uuid::Uuid;

use crate::{
    app_state::AppState,
    documents::import::ImportService,
    domain::BookSummary,
    errors::AppErrorDto,
    indexing::remote_cleanup::{RuntimeRemoteResourceCleaner, sweep_remote_resource_ids},
    maintenance::delete_book::DeleteBookService,
};

fn import_service(state: &AppState) -> ImportService {
    ImportService::with_maintenance_gate(
        state.db.pool().clone(),
        state.paths.clone(),
        state.import_cancellations.clone(),
        state.maintenance_gate.clone(),
    )
}

fn delete_service(state: &AppState) -> DeleteBookService {
    DeleteBookService::new(
        state.db.pool().clone(),
        state.paths.clone(),
        state.maintenance_gate.clone(),
    )
}

#[tauri::command]
pub async fn list_books(state: State<'_, AppState>) -> Result<Vec<BookSummary>, AppErrorDto> {
    import_service(&state)
        .list_books()
        .await
        .map_err(AppErrorDto::from)
}

#[tauri::command]
pub async fn get_book(
    state: State<'_, AppState>,
    book_id: Uuid,
) -> Result<BookSummary, AppErrorDto> {
    import_service(&state)
        .get_book(book_id)
        .await
        .map_err(AppErrorDto::from)
}

#[tauri::command]
pub async fn delete_failed_import(
    state: State<'_, AppState>,
    book_id: Uuid,
) -> Result<(), AppErrorDto> {
    let outcome = delete_service(&state)
        .delete_failed_book(book_id)
        .await
        .map_err(AppErrorDto::from)?;
    retry_detached_remote_cleanup(&state, outcome.detached_remote_resource_ids());
    Ok(())
}

#[tauri::command]
pub async fn delete_book(state: State<'_, AppState>, book_id: Uuid) -> Result<(), AppErrorDto> {
    let outcome = delete_service(&state)
        .delete_book(book_id)
        .await
        .map_err(AppErrorDto::from)?;
    retry_detached_remote_cleanup(&state, outcome.detached_remote_resource_ids());
    Ok(())
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

#[tauri::command]
pub async fn ensure_pdf_page_sections(
    state: State<'_, AppState>,
    book_id: Uuid,
    page_count: u32,
) -> Result<Vec<crate::book_repository::ReaderSection>, AppErrorDto> {
    crate::book_repository::ensure_pdf_page_sections(state.db.pool(), book_id, page_count)
        .await
        .map_err(AppErrorDto::from)
}

fn retry_detached_remote_cleanup(state: &AppState, resource_ids: &[Uuid]) {
    if resource_ids.is_empty() {
        return;
    }
    let Ok(permit) = state
        .maintenance_gate
        .try_acquire_normal(crate::domain::ActiveOperationKind::Indexing)
    else {
        return;
    };
    let pool = state.db.pool().clone();
    let credential_store = state.credential_store.clone();
    let cleaner = RuntimeRemoteResourceCleaner::new(
        credential_store.clone(),
        state.provider_capabilities.clone(),
    );
    let resource_ids = resource_ids.to_vec();
    tauri::async_runtime::spawn(async move {
        let _permit = permit;
        let _ = sweep_remote_resource_ids(
            &pool,
            credential_store.as_ref(),
            &cleaner,
            &resource_ids,
            tokio_util::sync::CancellationToken::new(),
        )
        .await;
    });
}
