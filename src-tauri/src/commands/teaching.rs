use std::{collections::HashMap, sync::OnceLock};

use parking_lot::Mutex;
use serde::Deserialize;
use tauri::{Emitter, State};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{
    app_state::AppState,
    db::teaching,
    domain::{TeachingInstructionDto, UpdateTeachingInstruction},
    errors::{AppErrorDto, AppResult},
    learning::teaching_test::{
        TeachingTestEvent, TeachingTestRequest, event_for_result, run_teaching_test,
    },
};

const TEACHING_TEST_EVENT: &str = "teaching-test-event";

static TEACHING_TEST_CANCELLATIONS: OnceLock<Mutex<HashMap<Uuid, (Uuid, CancellationToken)>>> =
    OnceLock::new();

#[tauri::command]
pub async fn get_teaching_instruction(
    state: State<'_, AppState>,
) -> Result<TeachingInstructionDto, AppErrorDto> {
    teaching::get_teaching_instruction(state.db.pool())
        .await
        .map_err(AppErrorDto::from)
}

#[tauri::command]
pub async fn update_teaching_instruction(
    state: State<'_, AppState>,
    update: UpdateTeachingInstruction,
) -> Result<TeachingInstructionDto, AppErrorDto> {
    persist_teaching_instruction(state.db.pool(), update)
        .await
        .map_err(AppErrorDto::from)
}

async fn persist_teaching_instruction(
    pool: &sqlx::SqlitePool,
    update: UpdateTeachingInstruction,
) -> AppResult<TeachingInstructionDto> {
    teaching::update_teaching_instruction(pool, update).await
}

#[tauri::command]
pub async fn start_teaching_test(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    request: TeachingTestRequest,
) -> Result<(), AppErrorDto> {
    let cancellation = replace_session_request(request.session_id, request.request_id);
    let pool = state.db.pool().clone();
    let runtime = crate::ai::runtime::ProviderRuntime::new(
        state.credential_store.clone(),
        state.provider_capabilities.clone(),
    );
    tauri::async_runtime::spawn(async move {
        let request_id = request.request_id;
        let result = run_teaching_test(&pool, &runtime, request, cancellation.clone(), |event| {
            emit_teaching_test_event(&app, event);
        })
        .await;
        if let Some(event) = event_for_result(request_id, result) {
            emit_teaching_test_event(&app, event);
        }
        remove_session_request(request_id);
    });
    Ok(())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CancelTeachingTestRequest {
    session_id: Uuid,
    request_id: Uuid,
}

#[tauri::command]
pub async fn cancel_teaching_test(request: CancelTeachingTestRequest) -> Result<(), AppErrorDto> {
    let registry = teaching_test_cancellations();
    if let Some((active_request_id, cancellation)) = registry.lock().get(&request.session_id)
        && *active_request_id == request.request_id
    {
        cancellation.cancel();
    }
    Ok(())
}

fn teaching_test_cancellations() -> &'static Mutex<HashMap<Uuid, (Uuid, CancellationToken)>> {
    TEACHING_TEST_CANCELLATIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn replace_session_request(session_id: Uuid, request_id: Uuid) -> CancellationToken {
    let cancellation = CancellationToken::new();
    let previous = teaching_test_cancellations()
        .lock()
        .insert(session_id, (request_id, cancellation.clone()));
    if let Some((_, previous)) = previous {
        previous.cancel();
    }
    cancellation
}

fn remove_session_request(request_id: Uuid) {
    teaching_test_cancellations()
        .lock()
        .retain(|_, (active_request_id, _)| *active_request_id != request_id);
}

fn emit_teaching_test_event(app: &tauri::AppHandle, event: TeachingTestEvent) {
    let _ = app.emit(TEACHING_TEST_EVENT, event);
}
