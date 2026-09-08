use sqlx::{Row, SqlitePool};

use crate::{
    ai::registry::ProviderCapabilityRegistry,
    domain::{
        AiOperation, CapabilitySupport, ProviderCapability, ProviderCapabilityRegistryDto,
        ProviderFileCapabilities, ProviderKind, ProviderModelCapability, ProviderProfileSummary,
    },
    errors::{AppError, AppResult},
};

/// Persisted discovery enables the UI; the adapter independently rechecks locality and vision
/// capability before sending any image to the runtime.
pub(crate) async fn model(
    pool: &SqlitePool,
    registry: &ProviderCapabilityRegistry,
    profile: &ProviderProfileSummary,
) -> AppResult<ProviderModelCapability> {
    let mut model = registry
        .model_for_profile(
            &profile.kind,
            &profile.model_id,
            profile.context_window_tokens,
        )
        .ok_or_else(AppError::unsupported_provider_capability)?;
    if profile.kind == ProviderKind::Ollama && profile.validated_at.is_some() {
        let vision: bool =
            sqlx::query_scalar("SELECT local_vision FROM provider_profiles WHERE id = ?")
                .bind(profile.id.to_string())
                .fetch_one(pool)
                .await?;
        if vision {
            model.image_input = CapabilitySupport::Supported;
            model.image_limits = Some(crate::ai::local::LOCAL_IMAGE_LIMITS);
        }
    }
    Ok(model)
}

pub(crate) async fn supports(
    pool: &SqlitePool,
    registry: &ProviderCapabilityRegistry,
    profile: &ProviderProfileSummary,
    operation: AiOperation,
) -> AppResult<bool> {
    if profile.kind.is_local() {
        return Ok(match operation {
            AiOperation::TextLearning => true,
            AiOperation::VisionLearning => {
                model(pool, registry, profile).await?.image_input == CapabilitySupport::Supported
            }
            AiOperation::StructuredPageAnalysis => false,
        });
    }
    let support = registry.operation_support(&profile.kind, &profile.model_id, operation);
    Ok(support == CapabilitySupport::Supported
        || (support == CapabilitySupport::Unknown
            && operation == AiOperation::TextLearning
            && profile.validated_at.is_some()))
}

pub(crate) async fn public_registry(
    pool: &SqlitePool,
    registry: &ProviderCapabilityRegistry,
) -> AppResult<ProviderCapabilityRegistryDto> {
    let mut result = registry.public_registry();
    let rows = sqlx::query("SELECT id FROM provider_profiles WHERE provider_kind IN ('ollama','lm_studio') ORDER BY created_at,id")
        .fetch_all(pool).await?;
    for row in rows {
        let id: String = row.try_get("id")?;
        let profile = super::providers::load_provider_profile_metadata(
            pool,
            id.parse()
                .map_err(|_| AppError::unsupported_provider_capability())?,
        )
        .await?;
        let model = model(pool, registry, &profile).await?;
        if let Some(provider) = result.providers.iter_mut().find(|p| p.kind == profile.kind) {
            if let Some(existing) = provider.models.iter_mut().find(|m| m.id == model.id) {
                // Registry DTOs are model keyed, while identities include the local port.
                // Advertise the conservative intersection when different services reuse a name.
                if model.image_input != CapabilitySupport::Supported {
                    existing.image_input = CapabilitySupport::Unsupported;
                    existing.image_limits = None;
                }
                existing.context_window_tokens = existing
                    .context_window_tokens
                    .min(model.context_window_tokens);
            } else {
                provider.models.push(model);
            }
        } else {
            result.providers.push(ProviderCapability {
                kind: profile.kind.clone(),
                display_name: profile.kind.display_name().to_owned(),
                default_model: profile.model_id.clone(),
                models: vec![model],
                file_capabilities: ProviderFileCapabilities {
                    file_extraction: false,
                    file_ocr: false,
                    max_file_bytes: None,
                },
            });
        }
    }
    Ok(result)
}
