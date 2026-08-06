use std::path::PathBuf;

use tauri::{AppHandle, State};

use crate::{
    app_state::AppState,
    domain::{
        ActiveOperationKind, BackupSummaryDto, ClearAllDataSummaryDto, MaintenanceErrorDto,
        MaintenanceStatusDto, RestoreBackupSummaryDto, StorageUsageDto,
    },
    maintenance::storage::{
        StorageError, StorageLayout, WindowsDirectoryLauncher, collect_storage_usage,
        open_canonical_app_data_directory,
    },
    maintenance::{
        archive::BackupService, clear_all::ClearAllDataService, restore::RestoreService,
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

#[tauri::command]
pub async fn create_local_backup(
    state: State<'_, AppState>,
    destination: String,
) -> Result<BackupSummaryDto, MaintenanceErrorDto> {
    BackupService::new(
        state.db.pool().clone(),
        state.paths.clone(),
        state.maintenance_gate.clone(),
    )
    .create_backup(PathBuf::from(destination))
    .await
    .map_err(MaintenanceErrorDto::from)
}

#[tauri::command]
pub async fn restore_local_backup(
    state: State<'_, AppState>,
    archive: String,
) -> Result<RestoreBackupSummaryDto, MaintenanceErrorDto> {
    RestoreService::new(
        state.paths.clone(),
        state.maintenance_gate.clone(),
        state.credential_store.clone(),
    )
    .stage_restore(PathBuf::from(archive))
    .await
    .map_err(MaintenanceErrorDto::from)
}

#[tauri::command]
pub async fn clear_all_textbooklens_data(
    state: State<'_, AppState>,
    confirmation: String,
) -> Result<ClearAllDataSummaryDto, MaintenanceErrorDto> {
    ClearAllDataService::new(
        state.db.pool().clone(),
        state.paths.clone(),
        state.maintenance_gate.clone(),
        state.credential_store.clone(),
    )
    .clear_all_data(confirmation)
    .await
    .map_err(MaintenanceErrorDto::from)
}

/// This command is intentionally separate from restore/clear: those commands first
/// finish their durable staging work, then the user explicitly crosses the restart
/// boundary from the UI.
#[tauri::command]
pub fn restart_application(app: AppHandle) {
    app.restart()
}
