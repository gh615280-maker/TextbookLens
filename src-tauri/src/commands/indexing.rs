use tauri::{
    AppHandle, Emitter, State,
    ipc::{InvokeBody, Request as IpcRequest, Response},
};
use uuid::Uuid;

use crate::{
    app_state::AppState,
    errors::{AppError, AppErrorCode, AppErrorDto},
    indexing::coordinator::{
        ConfirmIndexOperationRequest, IndexCoordinatorService, IndexingEventDto,
        RenderClaimBatchDto, RenderedPageCaptureMetadata, decode_rendered_submissions,
    },
};

const INDEXING_EVENT_NAME: &str = "textbooklens://indexing-progress";
const CAPTURE_METADATA_HEADER: &str = "x-textbooklens-index-captures";
const MAX_CAPTURE_METADATA_HEADER_BYTES: usize = 8 * 1024;

fn service(state: &AppState) -> IndexCoordinatorService {
    IndexCoordinatorService::new(
        state.db.pool().clone(),
        state.paths.clone(),
        state.indexing_operations.clone(),
        state.indexing_cancellations.clone(),
        state.provider_capabilities.clone(),
        state.credential_store.clone(),
    )
}

fn emit_event(app: &AppHandle, event: &IndexingEventDto) {
    let _ = app.emit(INDEXING_EVENT_NAME, event);
}

#[tauri::command]
pub async fn confirm_index_operation(
    state: State<'_, AppState>,
    request: ConfirmIndexOperationRequest,
) -> Result<Uuid, AppErrorDto> {
    service(&state)
        .confirm_operation(request)
        .await
        .map_err(AppErrorDto::from)
}

#[tauri::command]
pub async fn create_index_run(
    state: State<'_, AppState>,
    operation_token: Uuid,
    request: ConfirmIndexOperationRequest,
) -> Result<Uuid, AppErrorDto> {
    service(&state)
        .create_run(operation_token, request)
        .await
        .map_err(AppErrorDto::from)
}

#[tauri::command]
pub async fn authorize_index_run(
    state: State<'_, AppState>,
    run_id: Uuid,
    operation_token: Uuid,
    request: ConfirmIndexOperationRequest,
) -> Result<(), AppErrorDto> {
    service(&state)
        .authorize_run(run_id, operation_token, request)
        .await
        .map_err(AppErrorDto::from)
}

#[tauri::command]
pub async fn claim_index_render_batch(
    state: State<'_, AppState>,
) -> Result<RenderClaimBatchDto, AppErrorDto> {
    service(&state)
        .claim_render_batch()
        .await
        .map_err(AppErrorDto::from)
}

#[tauri::command]
pub async fn read_claimed_index_source(
    state: State<'_, AppState>,
    page_id: Uuid,
    attempt_id: Uuid,
) -> Result<Response, AppErrorDto> {
    service(&state)
        .read_claimed_source(page_id, attempt_id)
        .await
        .map(Response::new)
        .map_err(AppErrorDto::from)
}

#[tauri::command]
pub async fn submit_index_render_batch(
    app: AppHandle,
    state: State<'_, AppState>,
    ipc_request: IpcRequest<'_>,
) -> Result<Vec<IndexingEventDto>, AppErrorDto> {
    let submissions = decode_submission_request(&ipc_request).map_err(AppErrorDto::from)?;
    let received = service(&state)
        .submit_rendered_batch(submissions)
        .await
        .map_err(AppErrorDto::from)?;
    for event in &received.events {
        emit_event(&app, event);
    }
    Ok(received.events)
}

fn decode_submission_request(
    request: &IpcRequest<'_>,
) -> Result<Vec<crate::indexing::coordinator::RenderedPageSubmission>, AppError> {
    let metadata = request
        .headers()
        .get(CAPTURE_METADATA_HEADER)
        .ok_or_else(|| AppError::new(AppErrorCode::InvalidInput))?
        .to_str()
        .map_err(|_| AppError::new(AppErrorCode::InvalidInput))?;
    if metadata.is_empty() || metadata.len() > MAX_CAPTURE_METADATA_HEADER_BYTES {
        return Err(AppError::new(AppErrorCode::InvalidInput));
    }
    let metadata = serde_json::from_str::<Vec<RenderedPageCaptureMetadata>>(metadata)
        .map_err(|_| AppError::new(AppErrorCode::InvalidInput))?;
    let InvokeBody::Raw(body) = request.body() else {
        return Err(AppError::new(AppErrorCode::InvalidInput));
    };
    decode_rendered_submissions(metadata, body)
}

#[tauri::command]
pub async fn report_index_render_failure(
    app: AppHandle,
    state: State<'_, AppState>,
    page_id: Uuid,
    attempt_id: Uuid,
) -> Result<IndexingEventDto, AppErrorDto> {
    let event = service(&state)
        .report_render_failure(page_id, attempt_id)
        .await
        .map_err(AppErrorDto::from)?;
    emit_event(&app, &event);
    Ok(event)
}

#[tauri::command]
pub async fn pause_index_run(state: State<'_, AppState>, run_id: Uuid) -> Result<(), AppErrorDto> {
    service(&state)
        .pause_run(run_id)
        .await
        .map_err(AppErrorDto::from)
}

#[tauri::command]
pub async fn resume_index_run(state: State<'_, AppState>, run_id: Uuid) -> Result<(), AppErrorDto> {
    service(&state)
        .resume_run(run_id)
        .await
        .map_err(AppErrorDto::from)
}

#[tauri::command]
pub async fn cancel_index_run(state: State<'_, AppState>, run_id: Uuid) -> Result<(), AppErrorDto> {
    service(&state)
        .cancel_run(run_id)
        .await
        .map_err(AppErrorDto::from)
}

#[tauri::command]
pub async fn retry_index_page(
    state: State<'_, AppState>,
    page_id: Uuid,
    attempt_id: Uuid,
) -> Result<Uuid, AppErrorDto> {
    service(&state)
        .retry_page(page_id, attempt_id)
        .await
        .map_err(AppErrorDto::from)
}
