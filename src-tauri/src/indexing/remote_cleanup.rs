use std::{collections::HashSet, fmt, sync::Arc, time::Duration};

use async_trait::async_trait;
use chrono::Utc;
use secrecy::{ExposeSecret, SecretString};
use sqlx::{Row, SqlitePool};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{
    ai::{registry::ProviderCapabilityRegistry, runtime::ProviderRuntime},
    credentials::CredentialStore,
    db::indexing::{EncryptedRemoteResourceReference, database_timestamp, track_remote_resource},
    domain::{AiOperation, ProviderKind, RemoteCleanupHandle},
    errors::{AppError, AppErrorCode, AppResult},
};

const REFERENCE_PREFIX: &str = "enc:v1:keyring:";
const VAULT_KEY_PREFIX: &str = "textbooklens/remote-resource/";
const DELETED_MARKER: &str = "textbooklens:remote-resource-deleted:v1";
const MAX_CLEANUP_ATTEMPTS: i64 = 5;
const MAX_OPAQUE_REMOTE_ID_BYTES: usize = 4_096;
const CLEANUP_TIMEOUT: Duration = Duration::from_secs(30);
const REMOTE_SWEEP_LIMIT: i64 = 64;
const MAX_TARGETED_CLEANUP_IDS: usize = 10_001;

pub struct RemoteTrackingFailure {
    vault_key: String,
    error: AppError,
}

impl fmt::Debug for RemoteTrackingFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RemoteTrackingFailure")
            .field("vault_key", &"[REDACTED]")
            .field("error_code", &self.error.code)
            .finish()
    }
}

/// Places the opaque identifier in the OS vault before inserting only a random
/// keyring reference into SQLite.
pub async fn store_remote_handle(
    pool: &SqlitePool,
    credential_store: &dyn CredentialStore,
    page_id: Uuid,
    handle: &RemoteCleanupHandle,
) -> Result<Uuid, RemoteTrackingFailure> {
    let vault_id = Uuid::new_v4();
    let vault_key = vault_key(vault_id);
    let opaque = handle.opaque_id().expose_secret();
    if opaque.is_empty()
        || opaque.len() > MAX_OPAQUE_REMOTE_ID_BYTES
        || opaque.contains(['\0', '\r', '\n'])
    {
        return Err(RemoteTrackingFailure {
            vault_key,
            error: AppError::new(AppErrorCode::InvalidInput),
        });
    }
    credential_store
        .set(&vault_key, handle.opaque_id().clone())
        .await
        .map_err(|error| RemoteTrackingFailure {
            vault_key: vault_key.clone(),
            error,
        })?;
    let reference =
        match EncryptedRemoteResourceReference::new(format!("{REFERENCE_PREFIX}{vault_id}")) {
            Ok(reference) => reference,
            Err(error) => {
                return Err(RemoteTrackingFailure { vault_key, error });
            }
        };
    track_remote_resource(pool, page_id, reference)
        .await
        .map_err(|error| RemoteTrackingFailure { vault_key, error })
}

/// File-level variant of `store_remote_handle`; the opaque provider id still
/// lives only in the OS vault while SQLite records ownership and cleanup state.
pub async fn store_extraction_remote_handle(
    pool: &SqlitePool,
    credential_store: &dyn CredentialStore,
    extraction_id: Uuid,
    handle: &RemoteCleanupHandle,
) -> Result<Uuid, RemoteTrackingFailure> {
    let vault_id = Uuid::new_v4();
    let vault_key = vault_key(vault_id);
    let opaque = handle.opaque_id().expose_secret();
    if opaque.is_empty()
        || opaque.len() > MAX_OPAQUE_REMOTE_ID_BYTES
        || opaque.contains(['\0', '\r', '\n'])
    {
        return Err(RemoteTrackingFailure {
            vault_key,
            error: AppError::new(AppErrorCode::InvalidInput),
        });
    }
    credential_store
        .set(&vault_key, handle.opaque_id().clone())
        .await
        .map_err(|error| RemoteTrackingFailure {
            vault_key: vault_key.clone(),
            error,
        })?;
    let reference = EncryptedRemoteResourceReference::new(format!("{REFERENCE_PREFIX}{vault_id}"))
        .map_err(|error| RemoteTrackingFailure {
            vault_key: vault_key.clone(),
            error,
        })?;
    let owner = sqlx::query(
        "SELECT book_id, provider_profile_id, provider_kind FROM book_extractions WHERE id = ?",
    )
    .bind(extraction_id.to_string())
    .fetch_optional(pool)
    .await
    .map_err(|error| RemoteTrackingFailure {
        vault_key: vault_key.clone(),
        error: AppError::database(error),
    })?
    .ok_or_else(|| RemoteTrackingFailure {
        vault_key: vault_key.clone(),
        error: AppError::new(AppErrorCode::NotFound),
    })?;
    let resource_id = Uuid::new_v4();
    let timestamp = database_timestamp(Utc::now());
    sqlx::query("INSERT INTO provider_remote_resources (id, book_id, extraction_id, provider_profile_id, provider_kind, encrypted_reference, cleanup_status, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, 'pending', ?, ?)")
        .bind(resource_id.to_string())
        .bind(owner.try_get::<String, _>("book_id").map_err(|error| RemoteTrackingFailure { vault_key: vault_key.clone(), error: AppError::database(error) })?)
        .bind(extraction_id.to_string())
        .bind(owner.try_get::<Option<String>, _>("provider_profile_id").map_err(|error| RemoteTrackingFailure { vault_key: vault_key.clone(), error: AppError::database(error) })?)
        .bind(owner.try_get::<String, _>("provider_kind").map_err(|error| RemoteTrackingFailure { vault_key: vault_key.clone(), error: AppError::database(error) })?)
        .bind(reference.database_value()).bind(&timestamp).bind(&timestamp)
        .execute(pool).await
        .map_err(|error| RemoteTrackingFailure { vault_key, error: AppError::database(error) })?;
    Ok(resource_id)
}

pub async fn load_extraction_remote_handle(
    pool: &SqlitePool,
    credential_store: &dyn CredentialStore,
    extraction_id: Uuid,
) -> AppResult<Option<RemoteCleanupHandle>> {
    let row = sqlx::query("SELECT provider_kind, encrypted_reference FROM provider_remote_resources WHERE extraction_id = ? AND cleanup_status IN ('pending', 'failed', 'cleaning') ORDER BY created_at DESC LIMIT 1")
        .bind(extraction_id.to_string()).fetch_optional(pool).await?;
    let Some(row) = row else {
        return Ok(None);
    };
    let provider = parse_provider_kind(&row.try_get::<String, _>("provider_kind")?)?;
    let vault_id = parse_reference(&row.try_get::<String, _>("encrypted_reference")?)?;
    let opaque = credential_store.get(&vault_key(vault_id)).await?;
    Ok(Some(RemoteCleanupHandle::new(provider, opaque)))
}

/// Completes the compensating path after the SQLite insert failed. A failed
/// provider delete keeps the vault entry and never records a false success.
pub async fn finish_failed_tracking_compensation(
    credential_store: &dyn CredentialStore,
    failure: RemoteTrackingFailure,
    provider_delete_succeeded: bool,
) {
    if provider_delete_succeeded {
        let _ = credential_store.delete(&failure.vault_key).await;
    }
}

#[async_trait]
pub trait RemoteResourceCleaner: Send + Sync {
    async fn delete(
        &self,
        pool: &SqlitePool,
        profile_id: Uuid,
        handle: &RemoteCleanupHandle,
        cancel: CancellationToken,
    ) -> AppResult<()>;
}

pub struct RuntimeRemoteResourceCleaner {
    credential_store: Arc<dyn CredentialStore>,
    capabilities: ProviderCapabilityRegistry,
}

impl RuntimeRemoteResourceCleaner {
    pub fn new(
        credential_store: Arc<dyn CredentialStore>,
        capabilities: ProviderCapabilityRegistry,
    ) -> Self {
        Self {
            credential_store,
            capabilities,
        }
    }
}

#[async_trait]
impl RemoteResourceCleaner for RuntimeRemoteResourceCleaner {
    async fn delete(
        &self,
        pool: &SqlitePool,
        profile_id: Uuid,
        handle: &RemoteCleanupHandle,
        cancel: CancellationToken,
    ) -> AppResult<()> {
        ProviderRuntime::new(self.credential_store.clone(), self.capabilities.clone())
            .load(pool, profile_id, AiOperation::StructuredPageAnalysis)
            .await?
            .cleanup_remote_resource(handle, cancel)
            .await
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RemoteCleanupSummary {
    pub claimed: u32,
    pub succeeded: u32,
    pub failed: u32,
    pub recovered_success_markers: u32,
}

pub async fn sweep_remote_resources(
    pool: &SqlitePool,
    credential_store: &dyn CredentialStore,
    cleaner: &dyn RemoteResourceCleaner,
    cancel: CancellationToken,
) -> AppResult<RemoteCleanupSummary> {
    let mut summary = recover_success_markers(pool, credential_store).await?;
    let ids: Vec<String> = sqlx::query_scalar(
        "SELECT id FROM provider_remote_resources WHERE cleanup_status IN ('pending', 'failed') ORDER BY created_at, id LIMIT ?",
    )
    .bind(REMOTE_SWEEP_LIMIT)
    .fetch_all(pool)
    .await?;
    for id in ids {
        if cancel.is_cancelled() {
            break;
        }
        let resource_id = parse_uuid(&id)?;
        match cleanup_one(
            pool,
            credential_store,
            cleaner,
            resource_id,
            cancel.child_token(),
        )
        .await
        {
            Ok(CleanupOutcome::Succeeded) => {
                summary.claimed = summary.claimed.saturating_add(1);
                summary.succeeded = summary.succeeded.saturating_add(1);
            }
            Ok(CleanupOutcome::Failed) => {
                summary.claimed = summary.claimed.saturating_add(1);
                summary.failed = summary.failed.saturating_add(1);
            }
            Ok(CleanupOutcome::CompetingClaim) => {}
            Err(_) => {
                summary.claimed = summary.claimed.saturating_add(1);
                summary.failed = summary.failed.saturating_add(1);
            }
        }
    }
    Ok(summary)
}

/// Reconciles only a provider deletion that completed before the process
/// stopped but whose local success update did not. Startup must never turn a
/// durable pending/failed cleanup row into an implicit provider retry.
pub async fn recover_remote_cleanup_on_startup(
    pool: &SqlitePool,
    credential_store: &dyn CredentialStore,
) -> AppResult<RemoteCleanupSummary> {
    recover_success_markers(pool, credential_store).await
}

/// Retries only the detached opaque cleanup records returned by one local
/// textbook deletion. The identifiers are never serialized or logged, and a
/// provider failure remains represented by the durable row rather than
/// affecting the already committed local deletion.
pub async fn sweep_remote_resource_ids(
    pool: &SqlitePool,
    credential_store: &dyn CredentialStore,
    cleaner: &dyn RemoteResourceCleaner,
    resource_ids: &[Uuid],
    cancel: CancellationToken,
) -> AppResult<RemoteCleanupSummary> {
    if resource_ids.len() > MAX_TARGETED_CLEANUP_IDS {
        return Err(AppError::new(AppErrorCode::RequestConflict));
    }
    let mut seen = HashSet::with_capacity(resource_ids.len());
    if resource_ids.iter().any(|id| !seen.insert(*id)) {
        return Err(AppError::new(AppErrorCode::InvalidInput));
    }

    let mut summary = recover_success_markers_for_ids(pool, credential_store, resource_ids).await?;
    for resource_id in resource_ids {
        if cancel.is_cancelled() {
            break;
        }
        match cleanup_one(
            pool,
            credential_store,
            cleaner,
            *resource_id,
            cancel.child_token(),
        )
        .await
        {
            Ok(CleanupOutcome::Succeeded) => {
                summary.claimed = summary.claimed.saturating_add(1);
                summary.succeeded = summary.succeeded.saturating_add(1);
            }
            Ok(CleanupOutcome::Failed) => {
                summary.claimed = summary.claimed.saturating_add(1);
                summary.failed = summary.failed.saturating_add(1);
            }
            Ok(CleanupOutcome::CompetingClaim) => {}
            Err(error) if error.code == AppErrorCode::NotFound => {}
            Err(_) => {
                summary.claimed = summary.claimed.saturating_add(1);
                summary.failed = summary.failed.saturating_add(1);
            }
        }
    }
    Ok(summary)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CleanupOutcome {
    Succeeded,
    Failed,
    CompetingClaim,
}

async fn cleanup_one(
    pool: &SqlitePool,
    credential_store: &dyn CredentialStore,
    cleaner: &dyn RemoteResourceCleaner,
    resource_id: Uuid,
    cancel: CancellationToken,
) -> AppResult<CleanupOutcome> {
    let row = sqlx::query(
        "SELECT cleanup_status, cleanup_attempt_count FROM provider_remote_resources WHERE id = ?",
    )
    .bind(resource_id.to_string())
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::new(AppErrorCode::NotFound))?;
    let expected_status: String = row.try_get("cleanup_status")?;
    if !matches!(expected_status.as_str(), "pending" | "failed") {
        return Ok(CleanupOutcome::CompetingClaim);
    }
    let previous_attempts: i64 = row.try_get("cleanup_attempt_count")?;
    let attempt_id = Uuid::new_v4();
    let timestamp = database_timestamp(Utc::now());
    let claim = sqlx::query(
        "UPDATE provider_remote_resources SET cleanup_status = 'cleaning', cleanup_attempt_id = ?, cleanup_attempt_count = cleanup_attempt_count + 1, safe_error_code = NULL, safe_error_message = NULL, disposition_reason = NULL, last_cleanup_at = ?, updated_at = ? WHERE id = ? AND cleanup_status = ?",
    )
    .bind(attempt_id.to_string())
    .bind(&timestamp)
    .bind(&timestamp)
    .bind(resource_id.to_string())
    .bind(&expected_status)
    .execute(pool)
    .await?;
    if claim.rows_affected() == 0 {
        return Ok(CleanupOutcome::CompetingClaim);
    }
    if claim.rows_affected() != 1 {
        return Err(database_error());
    }
    if previous_attempts >= MAX_CLEANUP_ATTEMPTS {
        mark_failed(
            pool,
            resource_id,
            attempt_id,
            "REMOTE_RETRY_EXHAUSTED",
            "Remote cleanup retry limit reached.",
        )
        .await?;
        return Ok(CleanupOutcome::Failed);
    }

    let claimed = sqlx::query(
        "SELECT provider_profile_id, provider_kind, encrypted_reference FROM provider_remote_resources WHERE id = ? AND cleanup_status = 'cleaning' AND cleanup_attempt_id = ?",
    )
    .bind(resource_id.to_string())
    .bind(attempt_id.to_string())
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::new(AppErrorCode::RequestConflict))?;
    let Some(profile_id) = claimed.try_get::<Option<String>, _>("provider_profile_id")? else {
        mark_failed(
            pool,
            resource_id,
            attempt_id,
            "REMOTE_AUTH_REQUIRED",
            "Remote cleanup requires the original provider profile.",
        )
        .await?;
        return Ok(CleanupOutcome::Failed);
    };
    let profile_id = parse_uuid(&profile_id)?;
    let provider_kind = parse_provider_kind(&claimed.try_get::<String, _>("provider_kind")?)?;
    let vault_id = parse_reference(&claimed.try_get::<String, _>("encrypted_reference")?)?;
    let key = vault_key(vault_id);
    let opaque = match credential_store.get(&key).await {
        Ok(value) => value,
        Err(_) => {
            mark_failed(
                pool,
                resource_id,
                attempt_id,
                "REMOTE_AUTH_REQUIRED",
                "Remote cleanup reference is unavailable.",
            )
            .await?;
            return Ok(CleanupOutcome::Failed);
        }
    };
    if opaque.expose_secret() == DELETED_MARKER {
        mark_succeeded(pool, resource_id, attempt_id).await?;
        let _ = credential_store.delete(&key).await;
        return Ok(CleanupOutcome::Succeeded);
    }
    if cancel.is_cancelled() {
        mark_failed(
            pool,
            resource_id,
            attempt_id,
            "REMOTE_DELETE_FAILED",
            "Remote cleanup was cancelled before deletion.",
        )
        .await?;
        return Ok(CleanupOutcome::Failed);
    }

    let handle = RemoteCleanupHandle::new(provider_kind, opaque);
    let delete = tokio::time::timeout(
        CLEANUP_TIMEOUT,
        cleaner.delete(pool, profile_id, &handle, cancel.clone()),
    )
    .await
    .unwrap_or_else(|_| Err(AppError::new(AppErrorCode::ProviderUnavailable)));
    match delete {
        Ok(()) => {
            // The marker makes a later DB update failure recoverable without a
            // second provider delete. Failure to write it leaves the row in the
            // claimed state rather than falsely reporting success.
            credential_store
                .set(&key, SecretString::from(DELETED_MARKER))
                .await?;
            mark_succeeded(pool, resource_id, attempt_id).await?;
            let _ = credential_store.delete(&key).await;
            Ok(CleanupOutcome::Succeeded)
        }
        Err(error) => {
            let (code, message) = cleanup_failure(&error);
            mark_failed(pool, resource_id, attempt_id, code, message).await?;
            Ok(CleanupOutcome::Failed)
        }
    }
}

async fn recover_success_markers(
    pool: &SqlitePool,
    credential_store: &dyn CredentialStore,
) -> AppResult<RemoteCleanupSummary> {
    let rows = sqlx::query(
        "SELECT id, cleanup_attempt_id, encrypted_reference FROM provider_remote_resources WHERE cleanup_status = 'cleaning' ORDER BY updated_at, id LIMIT ?",
    )
    .bind(REMOTE_SWEEP_LIMIT)
    .fetch_all(pool)
    .await?;
    let mut recovered = 0_u32;
    for row in rows {
        let resource_id = parse_uuid(&row.try_get::<String, _>("id")?)?;
        let attempt_id = parse_uuid(&row.try_get::<String, _>("cleanup_attempt_id")?)?;
        let vault_id = parse_reference(&row.try_get::<String, _>("encrypted_reference")?)?;
        let key = vault_key(vault_id);
        if credential_store
            .get(&key)
            .await
            .is_ok_and(|value| value.expose_secret() == DELETED_MARKER)
        {
            mark_succeeded(pool, resource_id, attempt_id).await?;
            let _ = credential_store.delete(&key).await;
            recovered = recovered.saturating_add(1);
        }
    }
    Ok(RemoteCleanupSummary {
        recovered_success_markers: recovered,
        ..RemoteCleanupSummary::default()
    })
}

async fn recover_success_markers_for_ids(
    pool: &SqlitePool,
    credential_store: &dyn CredentialStore,
    resource_ids: &[Uuid],
) -> AppResult<RemoteCleanupSummary> {
    let mut recovered = 0_u32;
    for resource_id in resource_ids {
        let row = sqlx::query(
            "SELECT cleanup_attempt_id, encrypted_reference FROM provider_remote_resources WHERE id = ? AND cleanup_status = 'cleaning'",
        )
        .bind(resource_id.to_string())
        .fetch_optional(pool)
        .await?;
        let Some(row) = row else {
            continue;
        };
        let attempt_id = parse_uuid(&row.try_get::<String, _>("cleanup_attempt_id")?)?;
        let vault_id = parse_reference(&row.try_get::<String, _>("encrypted_reference")?)?;
        let key = vault_key(vault_id);
        if credential_store
            .get(&key)
            .await
            .is_ok_and(|value| value.expose_secret() == DELETED_MARKER)
        {
            mark_succeeded(pool, *resource_id, attempt_id).await?;
            let _ = credential_store.delete(&key).await;
            recovered = recovered.saturating_add(1);
        }
    }
    Ok(RemoteCleanupSummary {
        recovered_success_markers: recovered,
        ..RemoteCleanupSummary::default()
    })
}

async fn mark_succeeded(pool: &SqlitePool, resource_id: Uuid, attempt_id: Uuid) -> AppResult<()> {
    let timestamp = database_timestamp(Utc::now());
    let result = sqlx::query(
        "UPDATE provider_remote_resources SET cleanup_status = 'succeeded', safe_error_code = NULL, safe_error_message = NULL, disposition_reason = NULL, last_cleanup_at = ?, updated_at = ? WHERE id = ? AND cleanup_status = 'cleaning' AND cleanup_attempt_id = ?",
    )
    .bind(&timestamp)
    .bind(&timestamp)
    .bind(resource_id.to_string())
    .bind(attempt_id.to_string())
    .execute(pool)
    .await?;
    require_one(result.rows_affected())
}

async fn mark_failed(
    pool: &SqlitePool,
    resource_id: Uuid,
    attempt_id: Uuid,
    code: &'static str,
    message: &'static str,
) -> AppResult<()> {
    let timestamp = database_timestamp(Utc::now());
    let result = sqlx::query(
        "UPDATE provider_remote_resources SET cleanup_status = 'failed', safe_error_code = ?, safe_error_message = ?, disposition_reason = NULL, last_cleanup_at = ?, updated_at = ? WHERE id = ? AND cleanup_status = 'cleaning' AND cleanup_attempt_id = ?",
    )
    .bind(code)
    .bind(message)
    .bind(&timestamp)
    .bind(&timestamp)
    .bind(resource_id.to_string())
    .bind(attempt_id.to_string())
    .execute(pool)
    .await?;
    require_one(result.rows_affected())
}

pub async fn safely_dispose_failed_resource(
    pool: &SqlitePool,
    credential_store: &dyn CredentialStore,
    resource_id: Uuid,
    attempt_id: Uuid,
    user_authorized_forget: bool,
) -> AppResult<()> {
    if !user_authorized_forget {
        return Err(AppError::new(AppErrorCode::InvalidInput));
    }
    let reference: String = sqlx::query_scalar(
        "SELECT encrypted_reference FROM provider_remote_resources WHERE id = ? AND cleanup_status = 'failed' AND cleanup_attempt_id = ?",
    )
    .bind(resource_id.to_string())
    .bind(attempt_id.to_string())
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::new(AppErrorCode::RequestConflict))?;
    credential_store
        .delete(&vault_key(parse_reference(&reference)?))
        .await?;
    let timestamp = database_timestamp(Utc::now());
    let result = sqlx::query(
        "UPDATE provider_remote_resources SET cleanup_status = 'safely_disposed', safe_error_code = NULL, safe_error_message = NULL, disposition_reason = 'user_authorized_forget', last_cleanup_at = ?, updated_at = ? WHERE id = ? AND cleanup_status = 'failed' AND cleanup_attempt_id = ?",
    )
    .bind(&timestamp)
    .bind(&timestamp)
    .bind(resource_id.to_string())
    .bind(attempt_id.to_string())
    .execute(pool)
    .await?;
    require_one(result.rows_affected())
}

fn cleanup_failure(error: &AppError) -> (&'static str, &'static str) {
    match error.code {
        AppErrorCode::NotFound => (
            "REMOTE_NOT_FOUND",
            "The remote resource could not be confirmed deleted.",
        ),
        AppErrorCode::CredentialStoreError | AppErrorCode::InvalidApiKey => (
            "REMOTE_AUTH_REQUIRED",
            "Remote cleanup requires provider authorization.",
        ),
        _ => (
            "REMOTE_DELETE_FAILED",
            "The remote resource could not be deleted.",
        ),
    }
}

fn parse_reference(value: &str) -> AppResult<Uuid> {
    let suffix = value
        .strip_prefix(REFERENCE_PREFIX)
        .ok_or_else(database_error)?;
    if suffix.len() != 36 || value.len() != REFERENCE_PREFIX.len() + 36 {
        return Err(database_error());
    }
    parse_uuid(suffix)
}

fn vault_key(id: Uuid) -> String {
    format!("{VAULT_KEY_PREFIX}{id}")
}

fn parse_provider_kind(value: &str) -> AppResult<ProviderKind> {
    match value {
        "openai" => Ok(ProviderKind::OpenAi),
        "gemini" => Ok(ProviderKind::Gemini),
        "anthropic" => Ok(ProviderKind::Anthropic),
        "deepseek" => Ok(ProviderKind::DeepSeek),
        "kimi" => Ok(ProviderKind::Kimi),
        _ => Err(database_error()),
    }
}

fn parse_uuid(value: &str) -> AppResult<Uuid> {
    Uuid::parse_str(value).map_err(|_| database_error())
}

fn require_one(rows: u64) -> AppResult<()> {
    match rows {
        1 => Ok(()),
        0 => Err(AppError::new(AppErrorCode::RequestConflict)),
        _ => Err(database_error()),
    }
}

fn database_error() -> AppError {
    AppError::new(AppErrorCode::DatabaseError)
}
