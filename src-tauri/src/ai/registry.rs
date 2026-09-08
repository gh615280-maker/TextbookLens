use std::{collections::BTreeSet, fmt};

use chrono::NaiveDate;
use serde::Deserialize;

use crate::domain::{
    AiOperation, CapabilitySupport, ImageLimits, ProviderCapability, ProviderCapabilityRegistryDto,
    ProviderKind, ProviderModelCapability, ProviderProfileSummary,
};

const REGISTRY_JSON: &str = include_str!("../../resources/provider-models.json");
const MINIMUM_OUTPUT_TOKENS: u32 = 4_096;
const REGISTRY_SCHEMA_VERSION: u16 = 1;
const MAX_REGISTRY_IMAGES: u16 = 16;
const MAX_REGISTRY_IMAGE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_REGISTRY_TOTAL_IMAGE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_REGISTRY_IMAGE_DIMENSION: u32 = 8_192;
const MAX_REGISTRY_DECODED_PIXELS: u64 = 40_000_000;
pub const UNKNOWN_MODEL_CONTEXT_WINDOW_TOKENS: u32 = 32_000;

pub fn local_output_tokens(context: u32) -> u32 {
    // Reasoning and visible text share the generation allowance. The normal
    // 8192-token local context already reserves 4096 tokens for generation.
    if context >= 8_192 {
        4_096
    } else {
        (context / 4).clamp(256, 2_048)
    }
}

#[derive(Clone, Debug)]
pub struct ProviderCapabilityRegistry {
    schema_version: u16,
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
    schema_version: u16,
    providers: Vec<ProviderRecord>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProviderRecord {
    kind: ProviderKind,
    display_name: String,
    default_model: String,
    file_capabilities: RegistryFileCapabilities,
    models: Vec<ModelRecord>,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
struct RegistryFileCapabilities {
    file_extraction: bool,
    file_ocr: bool,
    max_file_bytes: Option<u64>,
}

impl From<RegistryFileCapabilities> for crate::domain::ProviderFileCapabilities {
    fn from(value: RegistryFileCapabilities) -> Self {
        Self {
            file_extraction: value.file_extraction,
            file_ocr: value.file_ocr,
            max_file_bytes: value.max_file_bytes,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ModelRecord {
    id: String,
    display_name: String,
    context_window_tokens: u32,
    default_max_output_tokens: u32,
    vendor_max_output_tokens: u32,
    text_chat: CapabilitySupport,
    image_input: CapabilitySupport,
    pdf_input: CapabilitySupport,
    strict_structured_output: CapabilitySupport,
    image_limits: Option<RegistryImageLimits>,
    last_verified: String,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
struct RegistryImageLimits {
    max_images: u16,
    max_encoded_bytes_each: u64,
    max_total_encoded_bytes: u64,
    max_dimension_px: u32,
    max_decoded_pixels_each: u64,
}

impl From<RegistryImageLimits> for ImageLimits {
    fn from(limits: RegistryImageLimits) -> Self {
        Self {
            max_images: limits.max_images,
            max_encoded_bytes_each: limits.max_encoded_bytes_each,
            max_total_encoded_bytes: limits.max_total_encoded_bytes,
            max_dimension_px: limits.max_dimension_px,
            max_decoded_pixels_each: limits.max_decoded_pixels_each,
        }
    }
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
            schema_version: document.schema_version,
            capabilities: document
                .providers
                .into_iter()
                .map(|provider| ProviderCapability {
                    kind: provider.kind,
                    display_name: provider.display_name,
                    default_model: provider.default_model,
                    file_capabilities: provider.file_capabilities.into(),
                    models: provider
                        .models
                        .into_iter()
                        .map(|model| ProviderModelCapability {
                            id: model.id,
                            display_name: model.display_name,
                            context_window_tokens: model.context_window_tokens,
                            default_max_output_tokens: model.default_max_output_tokens,
                            text_chat: model.text_chat,
                            image_input: model.image_input,
                            native_pdf_input: model.pdf_input,
                            pdf_input: model.pdf_input,
                            strict_structured_output: model.strict_structured_output,
                            image_limits: model.image_limits.map(ImageLimits::from),
                            last_verified: model.last_verified,
                        })
                        .collect(),
                })
                .collect(),
        })
    }

    pub fn capabilities(&self) -> &[ProviderCapability] {
        &self.capabilities
    }

    pub fn public_registry(&self) -> ProviderCapabilityRegistryDto {
        ProviderCapabilityRegistryDto {
            schema_version: self.schema_version,
            providers: self.capabilities.clone(),
        }
    }

    pub fn operation_support(
        &self,
        kind: &ProviderKind,
        model_id: &str,
        operation: AiOperation,
    ) -> CapabilitySupport {
        if kind.is_local() {
            return if operation == AiOperation::TextLearning {
                CapabilitySupport::Supported
            } else {
                CapabilitySupport::Unsupported
            };
        }
        let Some(model) = self.model(kind, model_id) else {
            return CapabilitySupport::Unknown;
        };

        match operation {
            AiOperation::TextLearning => model.text_chat,
            AiOperation::VisionLearning => combine_support(&[model.text_chat, model.image_input]),
            AiOperation::StructuredPageAnalysis => combine_support(&[
                model.text_chat,
                model.image_input,
                model.strict_structured_output,
            ]),
        }
    }

    pub fn resolve_operation(
        &self,
        operation: AiOperation,
        profiles: &[ProviderProfileSummary],
    ) -> Vec<ProviderProfileSummary> {
        profiles
            .iter()
            .filter(|profile| {
                let support = self.operation_support(&profile.kind, &profile.model_id, operation);
                support == CapabilitySupport::Supported
                    || (operation == AiOperation::TextLearning
                        && support == CapabilitySupport::Unknown
                        && profile.validated_at.is_some())
            })
            .cloned()
            .collect()
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

    fn model(&self, kind: &ProviderKind, model_id: &str) -> Option<&ProviderModelCapability> {
        self.capabilities
            .iter()
            .find(|capability| &capability.kind == kind)
            .and_then(|capability| capability.models.iter().find(|model| model.id == model_id))
    }

    /// Local capabilities come from validated installed models, never the cloud model catalog.
    pub fn model_for_profile(
        &self,
        kind: &ProviderKind,
        id: &str,
        context: u32,
    ) -> Option<ProviderModelCapability> {
        if kind.is_local() {
            return Some(ProviderModelCapability {
                id: id.to_owned(),
                display_name: id.to_owned(),
                context_window_tokens: context,
                default_max_output_tokens: local_output_tokens(context),
                text_chat: CapabilitySupport::Supported,
                image_input: CapabilitySupport::Unsupported,
                native_pdf_input: CapabilitySupport::Unsupported,
                pdf_input: CapabilitySupport::Unsupported,
                strict_structured_output: CapabilitySupport::Unsupported,
                image_limits: None,
                last_verified: String::new(),
            });
        }
        self.model(kind, id).cloned()
    }
}

fn validate(document: &RegistryDocument) -> Result<(), RegistryError> {
    if document.schema_version != REGISTRY_SCHEMA_VERSION {
        return Err(RegistryError::Invalid(format!(
            "registry schema_version must be {REGISTRY_SCHEMA_VERSION}"
        )));
    }

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
        let files = provider.file_capabilities;
        if files.file_ocr && !files.file_extraction
            || files.file_extraction != files.max_file_bytes.is_some()
            || files.max_file_bytes.is_some_and(|bytes| bytes == 0)
            || (provider.kind == ProviderKind::Kimi) != files.file_extraction
        {
            return Err(RegistryError::Invalid(
                "provider file extraction capabilities are inconsistent".to_owned(),
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
            if NaiveDate::parse_from_str(&model.last_verified, "%Y-%m-%d").is_err() {
                return Err(RegistryError::Invalid(
                    "model last_verified must be an ISO 8601 calendar date".to_owned(),
                ));
            }
            match (model.image_input, model.image_limits) {
                (CapabilitySupport::Supported, Some(limits)) => {
                    validate_image_limits(limits.into())?
                }
                (CapabilitySupport::Supported, None) => {
                    return Err(RegistryError::Invalid(
                        "models with supported image input require conservative image limits"
                            .to_owned(),
                    ));
                }
                (_, Some(_)) => {
                    return Err(RegistryError::Invalid(
                        "image limits are only valid for supported image input".to_owned(),
                    ));
                }
                (_, None) => {}
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

fn validate_image_limits(limits: ImageLimits) -> Result<(), RegistryError> {
    let possible_total = limits
        .max_encoded_bytes_each
        .checked_mul(u64::from(limits.max_images));
    let dimension_area =
        u64::from(limits.max_dimension_px).checked_mul(u64::from(limits.max_dimension_px));
    let invalid = limits.max_images == 0
        || limits.max_images > MAX_REGISTRY_IMAGES
        || limits.max_encoded_bytes_each == 0
        || limits.max_encoded_bytes_each > MAX_REGISTRY_IMAGE_BYTES
        || limits.max_total_encoded_bytes < limits.max_encoded_bytes_each
        || limits.max_total_encoded_bytes > MAX_REGISTRY_TOTAL_IMAGE_BYTES
        || possible_total.is_none_or(|total| limits.max_total_encoded_bytes > total)
        || limits.max_dimension_px == 0
        || limits.max_dimension_px > MAX_REGISTRY_IMAGE_DIMENSION
        || limits.max_decoded_pixels_each == 0
        || limits.max_decoded_pixels_each > MAX_REGISTRY_DECODED_PIXELS
        || dimension_area.is_none_or(|area| limits.max_decoded_pixels_each > area);
    if invalid {
        return Err(RegistryError::Invalid(
            "image limits must be positive, internally consistent, and within application ceilings"
                .to_owned(),
        ));
    }
    Ok(())
}

fn combine_support(values: &[CapabilitySupport]) -> CapabilitySupport {
    if values.contains(&CapabilitySupport::Unsupported) {
        CapabilitySupport::Unsupported
    } else if values
        .iter()
        .all(|value| *value == CapabilitySupport::Supported)
    {
        CapabilitySupport::Supported
    } else {
        CapabilitySupport::Unknown
    }
}
