use tauri::State;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{
    app_state::AppState,
    db::providers,
    domain::{
        ActiveOperationKind, AiOperation, ProviderCapabilityRegistryDto,
        ProviderOperationConsentCategory, ProviderOperationConsentDecision, ProviderProfileSummary,
    },
    errors::AppErrorDto,
    indexing::remote_cleanup::{RuntimeRemoteResourceCleaner, sweep_remote_resource_ids},
};

#[tauri::command]
pub fn list_provider_capabilities(state: State<'_, AppState>) -> ProviderCapabilityRegistryDto {
    state.provider_capabilities.public_registry()
}

#[tauri::command]
pub async fn list_provider_profiles(
    operation: Option<AiOperation>,
    state: State<'_, AppState>,
) -> Result<Vec<ProviderProfileSummary>, AppErrorDto> {
    let _permit = maintenance_permit(&state)?;
    let profiles =
        providers::list_provider_profiles(state.db.pool(), state.credential_store.as_ref())
            .await
            .map_err(AppErrorDto::from)?;
    Ok(match operation {
        Some(operation) => state
            .provider_capabilities
            .resolve_operation(operation, &profiles),
        None => profiles,
    })
}

#[tauri::command]
pub async fn set_active_provider_profile(
    profile_id: Uuid,
    state: State<'_, AppState>,
) -> Result<(), AppErrorDto> {
    let _permit = maintenance_permit(&state)?;
    providers::set_active_provider_profile(state.db.pool(), profile_id)
        .await
        .map_err(AppErrorDto::from)
}

#[tauri::command]
pub async fn delete_provider_profile(
    profile_id: Uuid,
    state: State<'_, AppState>,
) -> Result<(), AppErrorDto> {
    let _permit = maintenance_permit(&state)?;
    let ids: Vec<String> = sqlx::query_scalar("SELECT id FROM provider_remote_resources WHERE provider_profile_id = ? AND cleanup_status IN ('pending','failed') ORDER BY id")
        .bind(profile_id.to_string()).fetch_all(state.db.pool()).await.map_err(crate::errors::AppError::database)?;
    let ids = ids
        .into_iter()
        .map(|id| {
            Uuid::parse_str(&id).map_err(|_| {
                crate::errors::AppError::new(crate::errors::AppErrorCode::DatabaseError)
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut cleanup_failed = false;
    if !ids.is_empty() {
        let cleaner = RuntimeRemoteResourceCleaner::new(
            state.credential_store.clone(),
            state.provider_capabilities.clone(),
        );
        let summary = sweep_remote_resource_ids(
            state.db.pool(),
            state.credential_store.as_ref(),
            &cleaner,
            &ids,
            CancellationToken::new(),
        )
        .await?;
        cleanup_failed = summary.failed > 0;
    }
    let unresolved: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM provider_remote_resources WHERE provider_profile_id = ? AND cleanup_status IN ('pending','failed','cleaning')").bind(profile_id.to_string()).fetch_one(state.db.pool()).await.map_err(crate::errors::AppError::database)?;
    if cleanup_failed || unresolved > 0 {
        return Err(
            crate::errors::AppError::new(crate::errors::AppErrorCode::RequestConflict).into(),
        );
    }
    providers::delete_provider_profile(state.db.pool(), state.credential_store.clone(), profile_id)
        .await
        .map_err(AppErrorDto::from)
}

#[tauri::command]
pub async fn set_default_provider_profile(
    operation: AiOperation,
    profile_id: Uuid,
    state: State<'_, AppState>,
) -> Result<(), AppErrorDto> {
    let _permit = maintenance_permit(&state)?;
    providers::set_default_provider_profile(
        state.db.pool(),
        &state.provider_capabilities,
        operation,
        profile_id,
    )
    .await
    .map_err(AppErrorDto::from)
}

#[tauri::command]
pub async fn update_provider_operation_consent(
    profile_id: Uuid,
    category: ProviderOperationConsentCategory,
    decision: ProviderOperationConsentDecision,
    state: State<'_, AppState>,
) -> Result<(), AppErrorDto> {
    let _permit = maintenance_permit(&state)?;
    providers::update_provider_operation_consent(state.db.pool(), profile_id, category, decision)
        .await
        .map_err(AppErrorDto::from)
}

#[tauri::command]
pub async fn reset_provider_operation_consents(
    profile_id: Option<Uuid>,
    state: State<'_, AppState>,
) -> Result<(), AppErrorDto> {
    let _permit = maintenance_permit(&state)?;
    providers::reset_provider_operation_consents(state.db.pool(), profile_id)
        .await
        .map_err(AppErrorDto::from)
}

fn maintenance_permit(
    state: &AppState,
) -> Result<crate::maintenance::gate::NormalOperationPermit, AppErrorDto> {
    state
        .maintenance_gate
        .try_acquire_normal(ActiveOperationKind::Storage)
        .map_err(crate::errors::AppError::from)
        .map_err(AppErrorDto::from)
}
