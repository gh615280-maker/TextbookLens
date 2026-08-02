use chrono::{DateTime, Utc};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::{
    credentials::CredentialStore,
    domain::{ProviderKind, ProviderProfileSummary},
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
        "SELECT id, provider_kind, display_name, model_id, context_window_tokens, credential_key, is_active, validated_at FROM provider_profiles ORDER BY created_at, id",
    )
    .fetch_all(pool)
    .await
    .map_err(AppError::from)?;

    let mut profiles = Vec::with_capacity(rows.len());
    for row in rows {
        let id = parse_uuid(row.try_get("id")?)?;
        let stored_key: String = row.try_get("credential_key")?;
        let expected_key = credential_key(id);
        let credential_status =
            if stored_key == expected_key && store.get(&expected_key).await.is_ok() {
                CredentialStatus::Available
            } else {
                CredentialStatus::Missing
            };
        profiles.push(ProviderProfileSummary {
            id,
            kind: parse_provider_kind(row.try_get("provider_kind")?)?,
            display_name: row.try_get("display_name")?,
            model_id: row.try_get("model_id")?,
            context_window_tokens: row.try_get::<i64, _>("context_window_tokens")? as u32,
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
    let profile_exists: Option<String> =
        sqlx::query_scalar("SELECT id FROM provider_profiles WHERE id = ?")
            .bind(profile_id.to_string())
            .fetch_optional(&mut *transaction)
            .await?;
    if profile_exists.is_none() {
        return Err(AppError::new(AppErrorCode::NotFound));
    }

    sqlx::query("UPDATE provider_profiles SET is_active = 0 WHERE is_active = 1")
        .execute(&mut *transaction)
        .await?;
    sqlx::query("UPDATE provider_profiles SET is_active = 1 WHERE id = ?")
        .bind(profile_id.to_string())
        .execute(&mut *transaction)
        .await?;
    sqlx::query("UPDATE app_settings SET active_provider_profile_id = ? WHERE id = 1")
        .bind(profile_id.to_string())
        .execute(&mut *transaction)
        .await?;
    transaction.commit().await?;
    Ok(())
}

pub async fn delete_provider_profile(
    pool: &SqlitePool,
    store: &dyn CredentialStore,
    profile_id: Uuid,
) -> AppResult<()> {
    let row = sqlx::query(
        "SELECT credential_key, is_active, EXISTS(SELECT 1 FROM app_settings WHERE id = 1 AND active_provider_profile_id = provider_profiles.id) AS selected FROM provider_profiles WHERE id = ?",
    )
    .bind(profile_id.to_string())
    .fetch_optional(pool)
    .await
    .map_err(AppError::from)?
    .ok_or_else(|| AppError::new(AppErrorCode::NotFound))?;

    let expected_key = credential_key(profile_id);
    let stored_key: String = row.try_get("credential_key")?;
    if stored_key != expected_key {
        return Err(AppError::credential_store(
            "profile credential reference is invalid",
        ));
    }
    let was_active = row.try_get::<bool, _>("is_active")? || row.try_get::<bool, _>("selected")?;
    let old_secret = store.get(&expected_key).await?;
    store.delete(&expected_key).await?;

    let database_result: Result<(), sqlx::Error> = async {
        let mut transaction = pool.begin().await?;
        sqlx::query("DELETE FROM provider_profiles WHERE id = ?")
            .bind(profile_id.to_string())
            .execute(&mut *transaction)
            .await?;

        if was_active {
            sqlx::query("UPDATE provider_profiles SET is_active = 0")
                .execute(&mut *transaction)
                .await?;
            let replacement: Option<String> = sqlx::query_scalar(
                "SELECT id FROM provider_profiles ORDER BY created_at, id LIMIT 1",
            )
            .fetch_optional(&mut *transaction)
            .await?;
            if let Some(replacement) = &replacement {
                sqlx::query("UPDATE provider_profiles SET is_active = 1 WHERE id = ?")
                    .bind(replacement)
                    .execute(&mut *transaction)
                    .await?;
            }
            sqlx::query("UPDATE app_settings SET active_provider_profile_id = ? WHERE id = 1")
                .bind(replacement)
                .execute(&mut *transaction)
                .await?;
        }
        transaction.commit().await
    }
    .await;

    if let Err(error) = database_result {
        if store.set(&expected_key, old_secret).await.is_err() {
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
        return Err(AppError::database(error));
    }

    Ok(())
}

fn parse_uuid(value: String) -> AppResult<Uuid> {
    Uuid::parse_str(&value).map_err(AppError::database)
}

fn parse_timestamp(value: String) -> AppResult<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(&value)
        .map(|timestamp| timestamp.with_timezone(&Utc))
        .map_err(AppError::database)
}

fn parse_provider_kind(value: String) -> AppResult<ProviderKind> {
    match value.as_str() {
        "openai" => Ok(ProviderKind::OpenAi),
        "gemini" => Ok(ProviderKind::Gemini),
        "anthropic" => Ok(ProviderKind::Anthropic),
        "deepseek" => Ok(ProviderKind::DeepSeek),
        "kimi" => Ok(ProviderKind::Kimi),
        _ => Err(AppError::database("unknown provider kind")),
    }
}
