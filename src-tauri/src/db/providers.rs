use chrono::{DateTime, Utc};
use sqlx::{Row, Sqlite, SqlitePool, Transaction};
use uuid::Uuid;

use crate::{
    ai::registry::ProviderCapabilityRegistry,
    credentials::CredentialStore,
    domain::{
        AiOperation, CapabilitySupport, ProviderKind, ProviderOperationConsent,
        ProviderOperationConsentCategory, ProviderOperationConsentDecision, ProviderProfileSummary,
    },
    errors::{AppError, AppErrorCode, AppResult, SafeDiagnostic},
};

pub use crate::domain::CredentialStatus;

pub fn credential_key(profile_id: Uuid) -> String {
    format!("textbooklens/{profile_id}")
}

pub async fn list_provider_profiles(
    pool: &SqlitePool,
    store: &dyn CredentialStore,
) -> AppResult<Vec<ProviderProfileSummary>> {
    let rows = sqlx::query(
        "SELECT id, provider_kind, display_name, model_id, context_window_tokens, is_active, validated_at FROM provider_profiles ORDER BY created_at, id",
    )
    .fetch_all(pool)
    .await
    .map_err(AppError::from)?;

    let mut profiles = Vec::with_capacity(rows.len());
    for row in rows {
        let id = parse_uuid(row.try_get("id")?)?;
        let derived_key = credential_key(id);
        let credential_status = if store.get(&derived_key).await.is_ok() {
            CredentialStatus::Available
        } else {
            CredentialStatus::Missing
        };
        profiles.push(ProviderProfileSummary {
            id,
            kind: parse_provider_kind(row.try_get("provider_kind")?)?,
            display_name: row.try_get("display_name")?,
            model_id: row.try_get("model_id")?,
            context_window_tokens: parse_positive_u32(
                row.try_get::<i64, _>("context_window_tokens")?,
            )?,
            is_active: row.try_get("is_active")?,
            credential_status,
            validated_at: row
                .try_get::<Option<String>, _>("validated_at")?
                .map(parse_timestamp)
                .transpose()?,
        });
    }
    Ok(profiles)
}

pub async fn set_active_provider_profile(pool: &SqlitePool, profile_id: Uuid) -> AppResult<()> {
    let mut transaction = pool.begin().await?;
    ensure_profile_exists(&mut transaction, profile_id).await?;
    set_learning_default(&mut transaction, Some(profile_id)).await?;
    transaction.commit().await?;
    Ok(())
}

pub async fn set_default_provider_profile(
    pool: &SqlitePool,
    registry: &ProviderCapabilityRegistry,
    operation: AiOperation,
    profile_id: Uuid,
) -> AppResult<()> {
    let profile = load_profile_capability(pool, profile_id).await?;
    if !profile_supports_operation(registry, &profile, operation) {
        return Err(AppError::unsupported_provider_capability());
    }

    let mut transaction = pool.begin().await?;
    ensure_profile_exists(&mut transaction, profile_id).await?;
    match operation {
        AiOperation::TextLearning => {
            set_learning_default(&mut transaction, Some(profile_id)).await?;
        }
        AiOperation::VisionLearning | AiOperation::StructuredPageAnalysis => {
            let result =
                sqlx::query("UPDATE app_settings SET default_vision_profile_id = ? WHERE id = 1")
                    .bind(profile_id.to_string())
                    .execute(&mut *transaction)
                    .await?;
            if result.rows_affected() != 1 {
                return Err(AppError::new(AppErrorCode::DatabaseError));
            }
        }
    }
    transaction.commit().await?;
    Ok(())
}

pub async fn list_provider_operation_consents(
    pool: &SqlitePool,
    profile_id: Uuid,
) -> AppResult<Vec<ProviderOperationConsent>> {
    ensure_profile_exists_pool(pool, profile_id).await?;
    let rows = sqlx::query(
        "SELECT profile_id, category, decision, updated_at FROM provider_operation_consents WHERE profile_id = ? ORDER BY CASE category WHEN 'image_send' THEN 0 WHEN 'ai_index' THEN 1 WHEN 'cost_risk' THEN 2 END",
    )
    .bind(profile_id.to_string())
    .fetch_all(pool)
    .await?;
    if rows.len() != 3 {
        return Err(AppError::new(AppErrorCode::DatabaseError));
    }

    rows.into_iter()
        .map(|row| {
            let stored_profile_id = parse_uuid(row.try_get("profile_id")?)?;
            if stored_profile_id != profile_id {
                return Err(AppError::new(AppErrorCode::InvalidInput));
            }
            Ok(ProviderOperationConsent {
                profile_id: stored_profile_id,
                category: parse_consent_category(row.try_get("category")?)?,
                decision: parse_consent_decision(row.try_get("decision")?)?,
                updated_at: parse_timestamp(row.try_get("updated_at")?)?,
            })
        })
        .collect()
}

pub async fn update_provider_operation_consent(
    pool: &SqlitePool,
    profile_id: Uuid,
    category: ProviderOperationConsentCategory,
    decision: ProviderOperationConsentDecision,
) -> AppResult<()> {
    ensure_profile_exists_pool(pool, profile_id).await?;
    let result = sqlx::query(
        "INSERT INTO provider_operation_consents (profile_id, category, decision, updated_at) VALUES (?, ?, ?, ?) ON CONFLICT(profile_id, category) DO UPDATE SET decision = excluded.decision, updated_at = excluded.updated_at",
    )
    .bind(profile_id.to_string())
    .bind(consent_category_name(category))
    .bind(consent_decision_name(decision))
    .bind(Utc::now().to_rfc3339())
    .execute(pool)
    .await?;
    if result.rows_affected() != 1 {
        return Err(AppError::new(AppErrorCode::DatabaseError));
    }
    Ok(())
}

pub async fn reset_provider_operation_consents(
    pool: &SqlitePool,
    profile_id: Option<Uuid>,
) -> AppResult<()> {
    let mut transaction = pool.begin().await?;
    let profile_ids = match profile_id {
        Some(profile_id) => {
            ensure_profile_exists(&mut transaction, profile_id).await?;
            vec![profile_id]
        }
        None => sqlx::query_scalar::<_, String>(
            "SELECT id FROM provider_profiles ORDER BY created_at, id",
        )
        .fetch_all(&mut *transaction)
        .await?
        .into_iter()
        .map(parse_uuid)
        .collect::<AppResult<Vec<_>>>()?,
    };
    let updated_at = Utc::now().to_rfc3339();
    for target in profile_ids {
        for category in consent_categories() {
            sqlx::query(
                "INSERT INTO provider_operation_consents (profile_id, category, decision, updated_at) VALUES (?, ?, 'ask', ?) ON CONFLICT(profile_id, category) DO UPDATE SET decision = 'ask', updated_at = excluded.updated_at",
            )
            .bind(target.to_string())
            .bind(consent_category_name(category))
            .bind(&updated_at)
            .execute(&mut *transaction)
            .await?;
        }
    }
    transaction.commit().await?;
    Ok(())
}

pub async fn delete_provider_profile(
    pool: &SqlitePool,
    store: &dyn CredentialStore,
    profile_id: Uuid,
) -> AppResult<()> {
    let row = sqlx::query(
        "SELECT is_active, EXISTS(SELECT 1 FROM app_settings WHERE id = 1 AND active_provider_profile_id = provider_profiles.id) AS legacy_selected, EXISTS(SELECT 1 FROM app_settings WHERE id = 1 AND default_learning_profile_id = provider_profiles.id) AS learning_selected, EXISTS(SELECT 1 FROM app_settings WHERE id = 1 AND default_vision_profile_id = provider_profiles.id) AS vision_selected FROM provider_profiles WHERE id = ?",
    )
    .bind(profile_id.to_string())
    .fetch_optional(pool)
    .await
    .map_err(AppError::from)?
    .ok_or_else(|| AppError::new(AppErrorCode::NotFound))?;

    let derived_key = credential_key(profile_id);
    let old_secret = store.get(&derived_key).await?;
    store.delete(&derived_key).await?;

    let registry = ProviderCapabilityRegistry::load_embedded()
        .map_err(|_| AppError::new(AppErrorCode::DatabaseError))?;
    let learning_selected = row.try_get::<bool, _>("is_active")?
        || row.try_get::<bool, _>("legacy_selected")?
        || row.try_get::<bool, _>("learning_selected")?;
    let vision_selected = row.try_get::<bool, _>("vision_selected")?;

    let database_result: AppResult<()> = async {
        let mut transaction = pool.begin().await?;
        let result = sqlx::query("DELETE FROM provider_profiles WHERE id = ?")
            .bind(profile_id.to_string())
            .execute(&mut *transaction)
            .await?;
        if result.rows_affected() != 1 {
            return Err(AppError::new(AppErrorCode::NotFound));
        }

        if learning_selected {
            let fallback =
                first_compatible_profile(&mut transaction, &registry, AiOperation::TextLearning)
                    .await?;
            set_learning_default(&mut transaction, fallback).await?;
        }
        if vision_selected {
            let fallback = first_compatible_profile(
                &mut transaction,
                &registry,
                AiOperation::StructuredPageAnalysis,
            )
            .await?;
            sqlx::query("UPDATE app_settings SET default_vision_profile_id = ? WHERE id = 1")
                .bind(fallback.map(|id| id.to_string()))
                .execute(&mut *transaction)
                .await?;
        }
        transaction.commit().await?;
        Ok(())
    }
    .await;

    if let Err(error) = database_result {
        if store.set(&derived_key, old_secret).await.is_err() {
            let diagnostic = SafeDiagnostic::new(
                AppErrorCode::CredentialStoreError,
                "credential restoration failed; sensitive values omitted",
            );
            tracing::error!(
                diagnostic_id = %diagnostic.id,
                code = ?diagnostic.code,
                detail = %diagnostic.detail,
                "credential deletion compensation failed"
            );
        }
        return Err(error);
    }

    Ok(())
}

async fn set_learning_default(
    transaction: &mut Transaction<'_, Sqlite>,
    profile_id: Option<Uuid>,
) -> AppResult<()> {
    sqlx::query("UPDATE provider_profiles SET is_active = 0 WHERE is_active = 1")
        .execute(&mut **transaction)
        .await?;
    if let Some(profile_id) = profile_id {
        let result = sqlx::query("UPDATE provider_profiles SET is_active = 1 WHERE id = ?")
            .bind(profile_id.to_string())
            .execute(&mut **transaction)
            .await?;
        if result.rows_affected() != 1 {
            return Err(AppError::new(AppErrorCode::NotFound));
        }
    }
    let value = profile_id.map(|id| id.to_string());
    let result = sqlx::query(
        "UPDATE app_settings SET active_provider_profile_id = ?, default_learning_profile_id = ? WHERE id = 1",
    )
    .bind(&value)
    .bind(&value)
    .execute(&mut **transaction)
    .await?;
    if result.rows_affected() != 1 {
        return Err(AppError::new(AppErrorCode::DatabaseError));
    }
    Ok(())
}

async fn first_compatible_profile(
    transaction: &mut Transaction<'_, Sqlite>,
    registry: &ProviderCapabilityRegistry,
    operation: AiOperation,
) -> AppResult<Option<Uuid>> {
    let rows = sqlx::query(
        "SELECT id, provider_kind, model_id, validated_at FROM provider_profiles ORDER BY created_at, id",
    )
    .fetch_all(&mut **transaction)
    .await?;
    for row in rows {
        let profile = ProfileCapability {
            id: parse_uuid(row.try_get("id")?)?,
            kind: parse_provider_kind(row.try_get("provider_kind")?)?,
            model_id: row.try_get("model_id")?,
            validated_at: row
                .try_get::<Option<String>, _>("validated_at")?
                .map(parse_timestamp)
                .transpose()?,
        };
        if profile_supports_operation(registry, &profile, operation) {
            return Ok(Some(profile.id));
        }
    }
    Ok(None)
}

struct ProfileCapability {
    id: Uuid,
    kind: ProviderKind,
    model_id: String,
    validated_at: Option<DateTime<Utc>>,
}

async fn load_profile_capability(
    pool: &SqlitePool,
    profile_id: Uuid,
) -> AppResult<ProfileCapability> {
    let row = sqlx::query(
        "SELECT id, provider_kind, model_id, validated_at FROM provider_profiles WHERE id = ?",
    )
    .bind(profile_id.to_string())
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::new(AppErrorCode::NotFound))?;
    Ok(ProfileCapability {
        id: parse_uuid(row.try_get("id")?)?,
        kind: parse_provider_kind(row.try_get("provider_kind")?)?,
        model_id: row.try_get("model_id")?,
        validated_at: row
            .try_get::<Option<String>, _>("validated_at")?
            .map(parse_timestamp)
            .transpose()?,
    })
}

fn profile_supports_operation(
    registry: &ProviderCapabilityRegistry,
    profile: &ProfileCapability,
    operation: AiOperation,
) -> bool {
    match registry.operation_support(&profile.kind, &profile.model_id, operation) {
        CapabilitySupport::Supported => true,
        CapabilitySupport::Unknown => {
            operation == AiOperation::TextLearning && profile.validated_at.is_some()
        }
        CapabilitySupport::Unsupported => false,
    }
}

async fn ensure_profile_exists_pool(pool: &SqlitePool, profile_id: Uuid) -> AppResult<()> {
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM provider_profiles WHERE id = ?)")
            .bind(profile_id.to_string())
            .fetch_one(pool)
            .await?;
    if !exists {
        return Err(AppError::new(AppErrorCode::NotFound));
    }
    Ok(())
}

async fn ensure_profile_exists(
    transaction: &mut Transaction<'_, Sqlite>,
    profile_id: Uuid,
) -> AppResult<()> {
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM provider_profiles WHERE id = ?)")
            .bind(profile_id.to_string())
            .fetch_one(&mut **transaction)
            .await?;
    if !exists {
        return Err(AppError::new(AppErrorCode::NotFound));
    }
    Ok(())
}

fn parse_uuid(value: String) -> AppResult<Uuid> {
    Uuid::parse_str(&value).map_err(|_| AppError::new(AppErrorCode::DatabaseError))
}

fn parse_positive_u32(value: i64) -> AppResult<u32> {
    u32::try_from(value)
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| AppError::new(AppErrorCode::DatabaseError))
}

fn parse_timestamp(value: String) -> AppResult<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(&value)
        .map(|timestamp| timestamp.with_timezone(&Utc))
        .map_err(|_| AppError::new(AppErrorCode::DatabaseError))
}

fn parse_provider_kind(value: String) -> AppResult<ProviderKind> {
    match value.as_str() {
        "openai" => Ok(ProviderKind::OpenAi),
        "gemini" => Ok(ProviderKind::Gemini),
        "anthropic" => Ok(ProviderKind::Anthropic),
        "deepseek" => Ok(ProviderKind::DeepSeek),
        "kimi" => Ok(ProviderKind::Kimi),
        _ => Err(AppError::new(AppErrorCode::DatabaseError)),
    }
}

fn consent_categories() -> [ProviderOperationConsentCategory; 3] {
    [
        ProviderOperationConsentCategory::ImageSend,
        ProviderOperationConsentCategory::AiIndex,
        ProviderOperationConsentCategory::CostRisk,
    ]
}

fn consent_category_name(value: ProviderOperationConsentCategory) -> &'static str {
    match value {
        ProviderOperationConsentCategory::ImageSend => "image_send",
        ProviderOperationConsentCategory::AiIndex => "ai_index",
        ProviderOperationConsentCategory::CostRisk => "cost_risk",
    }
}

fn parse_consent_category(value: String) -> AppResult<ProviderOperationConsentCategory> {
    match value.as_str() {
        "image_send" => Ok(ProviderOperationConsentCategory::ImageSend),
        "ai_index" => Ok(ProviderOperationConsentCategory::AiIndex),
        "cost_risk" => Ok(ProviderOperationConsentCategory::CostRisk),
        _ => Err(AppError::new(AppErrorCode::DatabaseError)),
    }
}

fn consent_decision_name(value: ProviderOperationConsentDecision) -> &'static str {
    match value {
        ProviderOperationConsentDecision::Ask => "ask",
        ProviderOperationConsentDecision::SkipPrompt => "skip_prompt",
    }
}

fn parse_consent_decision(value: String) -> AppResult<ProviderOperationConsentDecision> {
    match value.as_str() {
        "ask" => Ok(ProviderOperationConsentDecision::Ask),
        "skip_prompt" => Ok(ProviderOperationConsentDecision::SkipPrompt),
        _ => Err(AppError::new(AppErrorCode::DatabaseError)),
    }
}

#[cfg(test)]
mod tests {
    use secrecy::SecretString;
    use tempfile::TempDir;

    use crate::{
        credentials::{CredentialStore, MemoryCredentialStore},
        db::Database,
        domain::{AiOperation, ProviderOperationConsentCategory, ProviderOperationConsentDecision},
        errors::AppErrorCode,
    };

    use super::*;

    #[test]
    fn providers_enforce_capability_defaults_and_profile_scoped_consents() {
        let temporary = TempDir::new().expect("temporary database");
        let database = Database::open(temporary.path().join("library.sqlite3")).expect("database");
        let registry = ProviderCapabilityRegistry::load_embedded().expect("embedded registry");
        let learning_id = Uuid::new_v4();
        let vision_id = Uuid::new_v4();

        tauri::async_runtime::block_on(async {
            insert_test_profile(
                database.pool(),
                learning_id,
                "deepseek",
                "deepseek-v4-flash",
            )
            .await;
            insert_test_profile(database.pool(), vision_id, "openai", "gpt-5.6").await;

            set_default_provider_profile(
                database.pool(),
                &registry,
                AiOperation::TextLearning,
                learning_id,
            )
            .await
            .expect("text default");
            let unsupported = set_default_provider_profile(
                database.pool(),
                &registry,
                AiOperation::StructuredPageAnalysis,
                learning_id,
            )
            .await
            .expect_err("text-only provider must not become the vision default");
            assert_eq!(unsupported.code, AppErrorCode::InvalidInput);
            assert_eq!(unsupported.stable_code(), "UNSUPPORTED_PROVIDER_CAPABILITY");
            set_default_provider_profile(
                database.pool(),
                &registry,
                AiOperation::StructuredPageAnalysis,
                vision_id,
            )
            .await
            .expect("vision default");

            let defaults: (Option<String>, Option<String>, Option<String>) = sqlx::query_as(
                "SELECT active_provider_profile_id, default_learning_profile_id, default_vision_profile_id FROM app_settings WHERE id = 1",
            )
            .fetch_one(database.pool())
            .await
            .expect("defaults");
            assert_eq!(
                defaults,
                (
                    Some(learning_id.to_string()),
                    Some(learning_id.to_string()),
                    Some(vision_id.to_string())
                )
            );

            update_provider_operation_consent(
                database.pool(),
                vision_id,
                ProviderOperationConsentCategory::ImageSend,
                ProviderOperationConsentDecision::SkipPrompt,
            )
            .await
            .expect("update consent");
            let learning_consents = list_provider_operation_consents(database.pool(), learning_id)
                .await
                .expect("learning consents");
            assert!(learning_consents.iter().all(|consent| {
                consent.decision == ProviderOperationConsentDecision::Ask
                    && consent.profile_id == learning_id
            }));
            let vision_consents = list_provider_operation_consents(database.pool(), vision_id)
                .await
                .expect("vision consents");
            assert_eq!(
                vision_consents
                    .iter()
                    .find(|consent| {
                        consent.category == ProviderOperationConsentCategory::ImageSend
                    })
                    .expect("image consent")
                    .decision,
                ProviderOperationConsentDecision::SkipPrompt
            );
            reset_provider_operation_consents(database.pool(), Some(vision_id))
                .await
                .expect("reset one profile");
            assert!(
                list_provider_operation_consents(database.pool(), vision_id)
                    .await
                    .expect("reset consents")
                    .iter()
                    .all(|consent| consent.decision == ProviderOperationConsentDecision::Ask)
            );
        });
    }

    #[test]
    fn providers_list_uses_only_the_uuid_derived_credential_key() {
        let temporary = TempDir::new().expect("temporary database");
        let database = Database::open(temporary.path().join("library.sqlite3")).expect("database");
        let store = MemoryCredentialStore::new();
        let profile_id = Uuid::new_v4();

        tauri::async_runtime::block_on(async {
            insert_test_profile(database.pool(), profile_id, "openai", "gpt-5.6").await;
            store
                .set(
                    &credential_key(profile_id),
                    SecretString::from("synthetic-profile-credential"),
                )
                .await
                .expect("synthetic credential");
            let profiles = list_provider_profiles(database.pool(), &store)
                .await
                .expect("profiles");
            assert_eq!(profiles.len(), 1);
            assert_eq!(profiles[0].credential_status, CredentialStatus::Available);
            let serialized = serde_json::to_string(&profiles).expect("safe metadata");
            assert!(!serialized.contains("credential_key"));
            assert!(!serialized.contains("credentialKey"));
            assert!(!serialized.contains("synthetic-profile-credential"));
        });
    }

    async fn insert_test_profile(pool: &SqlitePool, id: Uuid, provider_kind: &str, model_id: &str) {
        let timestamp = Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO provider_profiles (id, provider_kind, display_name, model_id, context_window_tokens, created_at, updated_at, validated_at) VALUES (?, ?, 'Synthetic provider', ?, 32000, ?, ?, ?)",
        )
        .bind(id.to_string())
        .bind(provider_kind)
        .bind(model_id)
        .bind(&timestamp)
        .bind(&timestamp)
        .bind(&timestamp)
        .execute(pool)
        .await
        .expect("insert profile");
        for category in consent_categories() {
            sqlx::query(
                "INSERT INTO provider_operation_consents (profile_id, category, decision, updated_at) VALUES (?, ?, 'ask', ?)",
            )
            .bind(id.to_string())
            .bind(consent_category_name(category))
            .bind(&timestamp)
            .execute(pool)
            .await
            .expect("insert consent");
        }
    }
}
