use tauri::State;
use uuid::Uuid;

use crate::{
    app_state::AppState,
    db::providers,
    domain::{ProviderCapability, ProviderProfileSummary},
    errors::AppErrorDto,
};

#[tauri::command]
pub fn list_provider_capabilities(state: State<'_, AppState>) -> Vec<ProviderCapability> {
    state.provider_capabilities.capabilities().to_vec()
}

#[tauri::command]
pub async fn list_provider_profiles(
    state: State<'_, AppState>,
) -> Result<Vec<ProviderProfileSummary>, AppErrorDto> {
    providers::list_provider_profiles(state.db.pool(), state.credential_store.as_ref())
        .await
        .map_err(AppErrorDto::from)
}

#[tauri::command]
pub async fn set_active_provider_profile(
    profile_id: Uuid,
    state: State<'_, AppState>,
) -> Result<(), AppErrorDto> {
    providers::set_active_provider_profile(state.db.pool(), profile_id)
        .await
        .map_err(AppErrorDto::from)
}

#[tauri::command]
pub async fn delete_provider_profile(
    profile_id: Uuid,
    state: State<'_, AppState>,
) -> Result<(), AppErrorDto> {
    providers::delete_provider_profile(state.db.pool(), state.credential_store.as_ref(), profile_id)
        .await
        .map_err(AppErrorDto::from)
}
