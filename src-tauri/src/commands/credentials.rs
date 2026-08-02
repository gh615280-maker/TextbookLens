use tauri::State;
use uuid::Uuid;

use crate::{
    app_state::AppState,
    db::providers::{self, ListedProviderProfile},
    errors::AppErrorDto,
};

#[tauri::command]
pub async fn list_provider_profiles(
    state: State<'_, AppState>,
) -> Result<Vec<ListedProviderProfile>, AppErrorDto> {
    providers::list_provider_profiles(state.db.pool(), state.credential_store.as_ref())
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
