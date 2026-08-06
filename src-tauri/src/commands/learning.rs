use std::sync::Arc;

use serde::Serialize;
use tauri::{
    State,
    ipc::{Channel, InvokeBody, Request as IpcRequest},
};
use uuid::Uuid;

use crate::{
    ai::runtime::ProviderRuntime,
    app_state::AppState,
    domain::{LearningRequestEvent, LearningRequestSnapshot},
    errors::{AppError, AppErrorCode},
    learning::{
        captures::{OwnedCaptureBytes, RegionCaptureMetadata},
        history::prepare_conversation_followup,
        orchestrator::LearningOrchestrator,
        preparation::{
            InvalidateLearningPreparations, LearningAuthorizationDecision,
            LearningPreparationService, PreparationSummary, PrepareLearningRequestMetadata,
        },
    },
};

const CAPTURE_METADATA_HEADER: &str = "x-textbooklens-learning-capture";
const MAX_CAPTURE_METADATA_HEADER_BYTES: usize = 4 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct LearningErrorDto {
    code: &'static str,
}

impl From<AppError> for LearningErrorDto {
    fn from(error: AppError) -> Self {
        Self {
            code: error.stable_code(),
        }
    }
}

fn service(state: &AppState) -> LearningPreparationService {
    LearningPreparationService::new(
        state.db.pool().clone(),
        state.learning_preparations.clone(),
        state.provider_capabilities.clone(),
        state.credential_store.clone(),
    )
}

fn runtime(state: &AppState) -> ProviderRuntime {
    ProviderRuntime::new(
        state.credential_store.clone(),
        state.provider_capabilities.clone(),
    )
}

fn orchestrator(state: &AppState) -> LearningOrchestrator {
    LearningOrchestrator::new(state.learning_requests.clone(), state.db.pool().clone())
}

#[tauri::command]
pub async fn prepare_learning_request(
    state: State<'_, AppState>,
    metadata: PrepareLearningRequestMetadata,
) -> Result<PreparationSummary, LearningErrorDto> {
    service(&state)
        .prepare(metadata)
        .await
        .map_err(LearningErrorDto::from)
}

#[tauri::command]
pub async fn authorize_learning_request(
    state: State<'_, AppState>,
    preparation_id: Uuid,
    decision: LearningAuthorizationDecision,
) -> Result<Option<Uuid>, LearningErrorDto> {
    service(&state)
        .authorize(preparation_id, decision)
        .await
        .map_err(LearningErrorDto::from)
}

#[tauri::command]
pub async fn stage_region_capture(
    state: State<'_, AppState>,
    ipc_request: IpcRequest<'_>,
) -> Result<(), LearningErrorDto> {
    let (metadata, bytes) = decode_capture_request(&ipc_request).map_err(LearningErrorDto::from)?;
    service(&state)
        .stage_region_capture(metadata, bytes)
        .await
        .map_err(LearningErrorDto::from)
}

#[tauri::command]
pub fn discard_learning_preparation(state: State<'_, AppState>, preparation_id: Uuid) {
    state.learning_preparations.discard(preparation_id);
}

#[tauri::command]
pub fn invalidate_learning_preparations(
    state: State<'_, AppState>,
    request: InvalidateLearningPreparations,
) -> Result<u32, LearningErrorDto> {
    state
        .learning_preparations
        .invalidate(request)
        .map_err(LearningErrorDto::from)
}

#[tauri::command]
pub async fn start_learning_request(
    state: State<'_, AppState>,
    preparation_id: Uuid,
) -> Result<LearningRequestSnapshot, LearningErrorDto> {
    let maintenance_permit = state
        .learning_requests
        .acquire_maintenance_permit()
        .map_err(LearningErrorDto::from)?;
    let prepared = service(&state)
        .consume_for_execution(preparation_id)
        .await
        .map_err(LearningErrorDto::from)?;
    orchestrator(&state)
        .start_prepared_with_maintenance_permit(&runtime(&state), prepared, maintenance_permit)
        .await
        .map_err(LearningErrorDto::from)
}

#[tauri::command]
pub fn subscribe_learning_request(
    state: State<'_, AppState>,
    request_id: Uuid,
    after_seq: u32,
    events: Channel<LearningRequestEvent>,
) -> Result<LearningRequestSnapshot, LearningErrorDto> {
    let subscriber_id = events.id();
    let sink = Arc::new(move |event| events.send(event).map_err(|_| ()));
    state
        .learning_requests
        .subscribe(request_id, after_seq, subscriber_id, sink)
        .map_err(LearningErrorDto::from)
}

#[tauri::command]
pub fn get_learning_request_snapshot(
    state: State<'_, AppState>,
    request_id: Uuid,
) -> Result<LearningRequestSnapshot, LearningErrorDto> {
    state
        .learning_requests
        .snapshot(request_id)
        .map_err(LearningErrorDto::from)
}

#[tauri::command]
pub fn cancel_learning_request(
    state: State<'_, AppState>,
    request_id: Uuid,
) -> Result<(), LearningErrorDto> {
    state
        .learning_requests
        .cancel(request_id)
        .map_err(LearningErrorDto::from)
}

#[tauri::command]
pub async fn start_conversation_followup(
    state: State<'_, AppState>,
    conversation_id: Uuid,
    question: String,
) -> Result<LearningRequestSnapshot, LearningErrorDto> {
    let maintenance_permit = state
        .learning_requests
        .acquire_maintenance_permit()
        .map_err(LearningErrorDto::from)?;
    let prepared = prepare_conversation_followup(
        state.db.pool(),
        &state.provider_capabilities,
        conversation_id,
        question,
    )
    .await
    .map_err(LearningErrorDto::from)?;
    orchestrator(&state)
        .start_followup_with_maintenance_permit(&runtime(&state), prepared, maintenance_permit)
        .await
        .map_err(LearningErrorDto::from)
}

fn decode_capture_request(
    request: &IpcRequest<'_>,
) -> Result<(RegionCaptureMetadata, OwnedCaptureBytes), AppError> {
    let metadata = request
        .headers()
        .get(CAPTURE_METADATA_HEADER)
        .ok_or_else(invalid_input)?
        .to_str()
        .map_err(|_| invalid_input())?;
    if metadata.is_empty() || metadata.len() > MAX_CAPTURE_METADATA_HEADER_BYTES {
        return Err(invalid_input());
    }
    let metadata =
        serde_json::from_str::<RegionCaptureMetadata>(metadata).map_err(|_| invalid_input())?;
    let InvokeBody::Raw(body) = request.body() else {
        return Err(invalid_input());
    };
    Ok((metadata, OwnedCaptureBytes::new(body.to_vec())))
}

fn invalid_input() -> AppError {
    AppError::new(AppErrorCode::InvalidInput)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn learning_error_dto_serializes_only_the_stable_code() {
        let serialized = serde_json::to_value(LearningErrorDto::from(AppError::new(
            AppErrorCode::InvalidInput,
        )))
        .unwrap();
        assert_eq!(serialized, serde_json::json!({ "code": "INVALID_INPUT" }));
    }
}
