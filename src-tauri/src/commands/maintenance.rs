use tauri::State;

use crate::{
    app_state::AppState,
    domain::{ActiveOperationKind, MaintenanceErrorDto, MaintenanceStatusDto, StorageUsageDto},
    maintenance::storage::{
        StorageError, StorageLayout, WindowsDirectoryLauncher, collect_storage_usage,
        open_canonical_app_data_directory,
    },
};

#[tauri::command]
pub fn get_maintenance_status(state: State<'_, AppState>) -> MaintenanceStatusDto {
    state.maintenance_gate.status()
}

#[tauri::command]
pub async fn get_storage_usage(
    state: State<'_, AppState>,
) -> Result<StorageUsageDto, MaintenanceErrorDto> {
    let permit = state
        .maintenance_gate
        .try_acquire_normal(ActiveOperationKind::Storage)
        .map_err(MaintenanceErrorDto::from)?;
    let layout = StorageLayout::from_app_paths(&state.paths).map_err(MaintenanceErrorDto::from)?;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        collect_storage_usage(&layout)
    })
    .await
    .map_err(|_| {
        MaintenanceErrorDto::from(StorageError {
            code: crate::domain::MaintenanceErrorCode::StorageScanFailed,
        })
    })?
    .map_err(MaintenanceErrorDto::from)
}

#[tauri::command]
pub fn open_app_data_directory(state: State<'_, AppState>) -> Result<(), MaintenanceErrorDto> {
    let _permit = state
        .maintenance_gate
        .try_acquire_normal(ActiveOperationKind::Storage)
        .map_err(MaintenanceErrorDto::from)?;
    open_canonical_app_data_directory(&state.paths.root, &WindowsDirectoryLauncher)
        .map_err(MaintenanceErrorDto::from)
}
