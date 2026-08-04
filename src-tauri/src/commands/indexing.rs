use serde::Deserialize;
use tauri::{
    AppHandle, Emitter, State,
    ipc::{InvokeBody, Request as IpcRequest, Response},
};
use uuid::Uuid;

use crate::{
    app_state::AppState,
    db::corrections::{
        CorrectionConflictDecision, DeleteIndexCorrection, ResolveIndexCorrectionConflict,
        SaveIndexCorrection, delete_index_correction as delete_index_correction_repository,
        list_page_corrections,
        resolve_index_correction_conflict as resolve_index_correction_conflict_repository,
        save_index_correction as save_index_correction_repository,
    },
    db::indexing::{
        find_current_run_aggregate_for_book, get_page_review, get_run_aggregate, list_page_reviews,
    },
    domain::{
        IndexCorrectionReviewDto, IndexCorrectionValueKind, IndexPageReviewDto,
        IndexRunAggregateDto,
    },
    errors::{AppError, AppErrorCode, AppErrorDto},
    indexing::coordinator::{
        ConfirmIndexOperationRequest, IndexCoordinatorService, IndexingEventDto,
        RenderClaimBatchDto, RenderedPageCaptureMetadata, decode_rendered_submissions,
    },
};

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SaveIndexCorrectionRequest {
    pub book_id: Uuid,
    pub page_id: Uuid,
    pub target_block_id: Uuid,
    pub target_content_version: u32,
    pub value_kind: IndexCorrectionValueKind,
    pub original_value_sha256: String,
    pub corrected_value: String,
    pub expected_revision: u32,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolveCorrectionDecisionRequest {
    Keep,
    Accept,
    Compare,
}

impl From<ResolveCorrectionDecisionRequest> for CorrectionConflictDecision {
    fn from(value: ResolveCorrectionDecisionRequest) -> Self {
        match value {
            ResolveCorrectionDecisionRequest::Keep => Self::Keep,
            ResolveCorrectionDecisionRequest::Accept => Self::Accept,
            ResolveCorrectionDecisionRequest::Compare => Self::Compare,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolveIndexCorrectionConflictRequest {
    pub book_id: Uuid,
    pub page_id: Uuid,
    pub correction_id: Uuid,
    pub target_content_version: u32,
    pub current_value_sha256: Option<String>,
    pub expected_revision: u32,
    pub decision: ResolveCorrectionDecisionRequest,
    pub compared_corrected_value: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeleteIndexCorrectionRequest {
    pub book_id: Uuid,
    pub page_id: Uuid,
    pub correction_id: Uuid,
    pub target_content_version: u32,
    pub current_value_sha256: Option<String>,
    pub expected_revision: u32,
}

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
    expected_updated_at: String,
) -> Result<(), AppErrorDto> {
    service(&state)
        .retry_page(page_id, &expected_updated_at)
        .await
        .map(|_| ())
        .map_err(AppErrorDto::from)
}

/// Returns only persisted, safe review metadata; it never reads a document source.
#[tauri::command]
pub async fn get_index_run_aggregate(
    state: State<'_, AppState>,
    run_id: Uuid,
) -> Result<IndexRunAggregateDto, AppErrorDto> {
    get_run_aggregate(state.db.pool(), run_id)
        .await
        .map_err(AppErrorDto::from)
}

/// Resolves only safe durable aggregate metadata for one canonical book UUID.
#[tauri::command]
pub async fn find_current_index_run_for_book(
    state: State<'_, AppState>,
    book_id: Uuid,
) -> Result<Option<IndexRunAggregateDto>, AppErrorDto> {
    find_current_run_aggregate_for_book(state.db.pool(), book_id)
        .await
        .map_err(AppErrorDto::from)
}

/// Lists only pages that require user attention, as determined by SQLite.
#[tauri::command]
pub async fn list_index_page_reviews(
    state: State<'_, AppState>,
    run_id: Uuid,
) -> Result<Vec<IndexPageReviewDto>, AppErrorDto> {
    list_page_reviews(state.db.pool(), run_id)
        .await
        .map_err(AppErrorDto::from)
}

#[tauri::command]
pub async fn get_index_page_review(
    state: State<'_, AppState>,
    page_id: Uuid,
) -> Result<IndexPageReviewDto, AppErrorDto> {
    get_page_review(state.db.pool(), page_id)
        .await
        .map_err(AppErrorDto::from)
}

#[tauri::command]
pub async fn list_index_page_corrections(
    state: State<'_, AppState>,
    page_id: Uuid,
) -> Result<Vec<IndexCorrectionReviewDto>, AppErrorDto> {
    list_page_corrections(state.db.pool(), page_id)
        .await
        .map_err(AppErrorDto::from)
}

#[tauri::command]
pub async fn save_index_correction(
    state: State<'_, AppState>,
    request: SaveIndexCorrectionRequest,
) -> Result<IndexCorrectionReviewDto, AppErrorDto> {
    save_index_correction_repository(
        state.db.pool(),
        SaveIndexCorrection {
            book_id: request.book_id,
            page_id: request.page_id,
            target_block_id: request.target_block_id,
            target_content_version: request.target_content_version,
            value_kind: request.value_kind,
            original_value_sha256: request.original_value_sha256,
            corrected_value: request.corrected_value,
            expected_revision: request.expected_revision,
        },
    )
    .await
    .map_err(AppErrorDto::from)
}

#[tauri::command]
pub async fn resolve_index_correction_conflict(
    state: State<'_, AppState>,
    request: ResolveIndexCorrectionConflictRequest,
) -> Result<Option<IndexCorrectionReviewDto>, AppErrorDto> {
    resolve_index_correction_conflict_repository(
        state.db.pool(),
        ResolveIndexCorrectionConflict {
            book_id: request.book_id,
            page_id: request.page_id,
            correction_id: request.correction_id,
            target_content_version: request.target_content_version,
            current_value_sha256: request.current_value_sha256,
            expected_revision: request.expected_revision,
            decision: request.decision.into(),
            compared_corrected_value: request.compared_corrected_value,
        },
    )
    .await
    .map_err(AppErrorDto::from)
}

#[tauri::command]
pub async fn delete_index_correction(
    state: State<'_, AppState>,
    request: DeleteIndexCorrectionRequest,
) -> Result<(), AppErrorDto> {
    delete_index_correction_repository(
        state.db.pool(),
        DeleteIndexCorrection {
            book_id: request.book_id,
            page_id: request.page_id,
            correction_id: request.correction_id,
            target_content_version: request.target_content_version,
            current_value_sha256: request.current_value_sha256,
            expected_revision: request.expected_revision,
        },
    )
    .await
    .map_err(AppErrorDto::from)
}
