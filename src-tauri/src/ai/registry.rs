use std::{collections::BTreeSet, fmt};

use serde::Deserialize;

use crate::domain::{ProviderCapability, ProviderKind, ProviderModelCapability};

const REGISTRY_JSON: &str = include_str!("../../resources/provider-models.json");
const MINIMUM_OUTPUT_TOKENS: u32 = 4_096;
pub const UNKNOWN_MODEL_CONTEXT_WINDOW_TOKENS: u32 = 32_000;

#[derive(Clone, Debug)]
pub struct ProviderCapabilityRegistry {
    capabilities: Vec<ProviderCapability>,
}

#[derive(Debug)]
pub enum RegistryError {
    Parse(serde_json::Error),
    Invalid(String),
}

impl fmt::Display for RegistryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse(error) => write!(formatter, "provider registry parse error: {error}"),
            Self::Invalid(message) => write!(formatter, "provider registry is invalid: {message}"),
        }
    }
}

impl std::error::Error for RegistryError {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RegistryDocument {
    providers: Vec<ProviderRecord>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProviderRecord {
    kind: ProviderKind,
    display_name: String,
    default_model: String,
    models: Vec<ModelRecord>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ModelRecord {
    id: String,
    display_name: String,
    context_window_tokens: u32,
    default_max_output_tokens: u32,
    vendor_max_output_tokens: u32,
}

impl ProviderCapabilityRegistry {
    pub fn load_embedded() -> Result<Self, RegistryError> {
        Self::from_json(REGISTRY_JSON)
    }

    pub fn from_json(json: &str) -> Result<Self, RegistryError> {
        let document =
            serde_json::from_str::<RegistryDocument>(json).map_err(RegistryError::Parse)?;
        validate(&document)?;
        Ok(Self {
            capabilities: document
                .providers
                .into_iter()
                .map(|provider| ProviderCapability {
                    kind: provider.kind,
                    display_name: provider.display_name,
                    default_model: provider.default_model,
                    models: provider
                        .models
                        .into_iter()
                        .map(|model| ProviderModelCapability {
                            id: model.id,
                            display_name: model.display_name,
                            context_window_tokens: model.context_window_tokens,
                            default_max_output_tokens: model.default_max_output_tokens,
                        })
                        .collect(),
                })
                .collect(),
        })
    }

    pub fn capabilities(&self) -> &[ProviderCapability] {
        &self.capabilities
    }

    pub fn resolve_context_window(
        &self,
        kind: &ProviderKind,
        model_id: &str,
        explicitly_supplied: Option<u32>,
    ) -> u32 {
        explicitly_supplied
            .filter(|tokens| *tokens > 0)
            .or_else(|| {
                self.capabilities
                    .iter()
                    .find(|capability| &capability.kind == kind)
                    .and_then(|capability| {
                        capability
                            .models
                            .iter()
                            .find(|model| model.id == model_id)
                            .map(|model| model.context_window_tokens)
                    })
            })
            .unwrap_or(UNKNOWN_MODEL_CONTEXT_WINDOW_TOKENS)
    }
}

fn validate(document: &RegistryDocument) -> Result<(), RegistryError> {
    let expected_kinds = BTreeSet::from([
        ProviderKind::OpenAi,
        ProviderKind::Gemini,
        ProviderKind::Anthropic,
        ProviderKind::DeepSeek,
        ProviderKind::Kimi,
    ]);
    let actual_kinds = document
        .providers
        .iter()
        .map(|provider| provider.kind.clone())
        .collect::<BTreeSet<_>>();
    if document.providers.len() != expected_kinds.len() || actual_kinds != expected_kinds {
        return Err(RegistryError::Invalid(
            "exactly one record is required for each supported provider kind".to_owned(),
        ));
    }

    for provider in &document.providers {
        if provider.display_name.trim().is_empty() || provider.default_model.trim().is_empty() {
            return Err(RegistryError::Invalid(
                "provider display names and default model IDs must not be blank".to_owned(),
            ));
        }
        let mut model_ids = BTreeSet::new();
        for model in &provider.models {
            if model.id.trim().is_empty() || model.display_name.trim().is_empty() {
                return Err(RegistryError::Invalid(
                    "model IDs and display names must not be blank".to_owned(),
                ));
            }
            if !model_ids.insert(model.id.as_str()) {
                return Err(RegistryError::Invalid(
                    "model IDs must be unique within a provider".to_owned(),
                ));
            }
            if model.context_window_tokens == 0 {
                return Err(RegistryError::Invalid(
                    "model context windows must be positive".to_owned(),
                ));
            }
            if model.default_max_output_tokens < MINIMUM_OUTPUT_TOKENS
                || model.default_max_output_tokens > model.vendor_max_output_tokens
            {
                return Err(RegistryError::Invalid(
                    "default output limits must be at least 4096 and no larger than the verified vendor maximum"
                        .to_owned(),
                ));
            }
        }
        if model_ids
            .iter()
            .filter(|id| **id == provider.default_model)
            .count()
            != 1
        {
            return Err(RegistryError::Invalid(
                "each provider default model must appear exactly once in its models".to_owned(),
            ));
        }
    }
    Ok(())
}
