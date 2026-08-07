use std::sync::Arc;

use secrecy::SecretString;
use serde::Deserialize;
use sqlx::SqlitePool;
use tauri::State;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{
    ai::registry::ProviderCapabilityRegistry,
    ai::{
        provider::{validate_credential, validate_display_name},
        runtime::ProviderRuntime,
    },
    app_state::AppState,
    credentials::CredentialStore,
    db::providers::{self, NewProviderProfile, ReplacementProviderCredential},
    domain::{ActiveOperationKind, AiOperation, ProviderKind, ProviderProfileSummary},
    errors::{AppErrorDto, AppResult},
    indexing::remote_cleanup::{RuntimeRemoteResourceCleaner, sweep_remote_resource_ids},
};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SaveProviderProfileRequest {
    provider_kind: ProviderKind,
    display_name: String,
    model_id: Option<String>,
    credential: String,
}

#[tauri::command]
pub async fn validate_and_save_provider_profile(
    request: SaveProviderProfileRequest,
    state: State<'_, AppState>,
) -> Result<ProviderProfileSummary, AppErrorDto> {
    let _permit = state
        .maintenance_gate
        .try_acquire_normal(ActiveOperationKind::Storage)
        .map_err(crate::errors::AppError::from)
        .map_err(AppErrorDto::from)?;
    let store = state.credential_store.clone();
    let runtime = ProviderRuntime::new(store.clone(), state.provider_capabilities.clone());
    validate_and_save_provider_profile_impl(request, state.db.pool(), store, &runtime)
        .await
        .map_err(AppErrorDto::from)
}

#[tauri::command]
pub async fn replace_provider_profile_credential(
    profile_id: Uuid,
    credential: String,
    state: State<'_, AppState>,
) -> Result<ProviderProfileSummary, AppErrorDto> {
    let _permit = state
        .maintenance_gate
        .try_acquire_normal(ActiveOperationKind::Storage)
        .map_err(crate::errors::AppError::from)
        .map_err(AppErrorDto::from)?;
    let store = state.credential_store.clone();
    let runtime = ProviderRuntime::new(store.clone(), state.provider_capabilities.clone());
    replace_provider_profile_credential_impl(
        profile_id,
        SecretString::from(credential),
        state.db.pool(),
        store,
        &runtime,
    )
    .await
    .map_err(AppErrorDto::from)
}

pub(crate) async fn validate_and_save_provider_profile_impl(
    request: SaveProviderProfileRequest,
    pool: &SqlitePool,
    store: Arc<dyn CredentialStore>,
    runtime: &ProviderRuntime,
) -> AppResult<ProviderProfileSummary> {
    let SaveProviderProfileRequest {
        provider_kind,
        display_name,
        model_id,
        credential,
    } = request;
    validate_display_name(&display_name).map_err(|error| error.into_app_error())?;
    let credential = SecretString::from(credential);
    validate_credential(&credential).map_err(|error| error.into_app_error())?;
    let guard = providers::try_provider_mutation(pool).await?;
    let validated = runtime
        .validate_candidate(
            provider_kind,
            model_id.as_deref(),
            credential,
            AiOperation::TextLearning,
        )
        .await?;
    providers::insert_validated_provider_profile(
        pool,
        store,
        guard,
        NewProviderProfile {
            kind: validated.kind,
            display_name,
            model_id: validated.model_id,
            context_window_tokens: validated.context_window_tokens,
            credential: validated.credential,
            validated_at: validated.validated_at,
        },
    )
    .await
}

pub(crate) async fn replace_provider_profile_credential_impl(
    profile_id: Uuid,
    credential: SecretString,
    pool: &SqlitePool,
    store: Arc<dyn CredentialStore>,
    runtime: &ProviderRuntime,
) -> AppResult<ProviderProfileSummary> {
    validate_credential(&credential).map_err(|error| error.into_app_error())?;
    let guard = providers::try_provider_mutation(pool).await?;
    let profile = providers::load_provider_profile_metadata(pool, profile_id).await?;
    let validated = runtime
        .validate_candidate(
            profile.kind.clone(),
            Some(&profile.model_id),
            credential,
            AiOperation::TextLearning,
        )
        .await?;
    let resource_ids: Vec<String> = sqlx::query_scalar("SELECT id FROM provider_remote_resources WHERE provider_profile_id = ? AND cleanup_status IN ('pending','failed') ORDER BY id")
        .bind(profile_id.to_string()).fetch_all(pool).await?;
    let resource_ids = resource_ids
        .into_iter()
        .map(|id| {
            Uuid::parse_str(&id).map_err(|_| {
                crate::errors::AppError::new(crate::errors::AppErrorCode::DatabaseError)
            })
        })
        .collect::<AppResult<Vec<_>>>()?;
    let mut cleanup_failed = false;
    if !resource_ids.is_empty() {
        let registry = ProviderCapabilityRegistry::load_embedded().map_err(|_| {
            crate::errors::AppError::new(crate::errors::AppErrorCode::DatabaseError)
        })?;
        let cleaner = RuntimeRemoteResourceCleaner::new(store.clone(), registry);
        let cleanup = sweep_remote_resource_ids(
            pool,
            store.as_ref(),
            &cleaner,
            &resource_ids,
            CancellationToken::new(),
        )
        .await?;
        cleanup_failed = cleanup.failed > 0;
    }
    let unresolved: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM provider_remote_resources WHERE provider_profile_id = ? AND cleanup_status IN ('pending','failed','cleaning')").bind(profile_id.to_string()).fetch_one(pool).await?;
    if cleanup_failed || unresolved > 0 {
        return Err(crate::errors::AppError::new(
            crate::errors::AppErrorCode::RequestConflict,
        ));
    }
    providers::replace_validated_provider_credential(
        pool,
        store,
        guard,
        ReplacementProviderCredential {
            profile,
            context_window_tokens: validated.context_window_tokens,
            credential: validated.credential,
            validated_at: validated.validated_at,
        },
    )
    .await
}

#[cfg(test)]
impl SaveProviderProfileRequest {
    pub(crate) fn synthetic(
        provider_kind: ProviderKind,
        display_name: &str,
        model_id: Option<&str>,
        credential: &str,
    ) -> Self {
        Self {
            provider_kind,
            display_name: display_name.to_owned(),
            model_id: model_id.map(str::to_owned),
            credential: credential.to_owned(),
        }
    }
}
