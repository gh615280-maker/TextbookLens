use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "provider.ts")]
pub enum ProviderKind {
    #[serde(rename = "openai")]
    OpenAi,
    Gemini,
    Anthropic,
    #[serde(rename = "deepseek")]
    DeepSeek,
    Kimi,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "provider.ts")]
pub struct ProviderModelCapability {
    pub id: String,
    pub display_name: String,
    pub context_window_tokens: u32,
    pub default_max_output_tokens: u32,
    pub text_chat: CapabilitySupport,
    pub image_input: CapabilitySupport,
    pub pdf_input: CapabilitySupport,
    pub strict_structured_output: CapabilitySupport,
    pub image_limits: Option<ImageLimits>,
    pub last_verified: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "provider.ts")]
pub enum CapabilitySupport {
    Supported,
    Unsupported,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "provider.ts")]
pub enum AiOperation {
    TextLearning,
    VisionLearning,
    StructuredPageAnalysis,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "provider.ts")]
pub struct ImageLimits {
    pub max_images: u16,
    pub max_encoded_bytes_each: u64,
    pub max_total_encoded_bytes: u64,
    pub max_dimension_px: u32,
    pub max_decoded_pixels_each: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "provider.ts")]
pub struct ProviderCapability {
    pub kind: ProviderKind,
    pub display_name: String,
    pub default_model: String,
    pub models: Vec<ProviderModelCapability>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "provider.ts")]
pub struct ProviderCapabilityRegistryDto {
    pub schema_version: u16,
    pub providers: Vec<ProviderCapability>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "provider.ts")]
pub enum CredentialStatus {
    Available,
    Missing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "provider.ts")]
pub enum ProviderOperationConsentCategory {
    ImageSend,
    AiIndex,
    CostRisk,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "provider.ts")]
pub enum ProviderOperationConsentDecision {
    Ask,
    SkipPrompt,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "provider.ts")]
pub struct ProviderOperationConsent {
    pub profile_id: Uuid,
    pub category: ProviderOperationConsentCategory,
    pub decision: ProviderOperationConsentDecision,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "provider.ts")]
pub struct ValidationResult {
    pub model: String,
    pub context_window_tokens: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "provider.ts")]
pub enum UnifiedRole {
    User,
    Assistant,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "provider.ts")]
pub struct UnifiedMessage {
    pub role: UnifiedRole,
    pub content: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "provider.ts")]
pub struct UnifiedChatRequest {
    pub model: String,
    pub system: String,
    pub messages: Vec<UnifiedMessage>,
    pub max_output_tokens: u32,
    pub expected_language: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export_to = "provider.ts")]
pub enum UnifiedStreamEvent {
    TextDelta {
        text: String,
    },
    Usage {
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
    },
    Completed,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "provider.ts")]
pub struct ProviderProfileSummary {
    pub id: Uuid,
    pub kind: ProviderKind,
    pub display_name: String,
    pub model_id: String,
    pub context_window_tokens: u32,
    pub is_active: bool,
    pub credential_status: CredentialStatus,
    pub validated_at: Option<DateTime<Utc>>,
}
