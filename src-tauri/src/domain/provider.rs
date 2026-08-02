use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "provider.ts")]
pub enum ProviderKind {
    Openai,
    Gemini,
    Anthropic,
    Deepseek,
    Kimi,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "provider.ts")]
pub struct ValidationResult {
    pub model: String,
    pub context_window_tokens: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "provider.ts")]
pub struct UnifiedChatRequest {
    pub system_instruction: String,
    pub model: String,
    pub target_language: String,
    pub messages: Vec<ChatMessage>,
    pub context: Vec<ContextCitation>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "provider.ts")]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "provider.ts")]
pub struct ContextCitation {
    pub section_id: Uuid,
    pub locator_json: String,
    pub text: String,
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
pub struct ProviderProfileDto {
    pub id: Uuid,
    pub provider_kind: ProviderKind,
    pub display_name: String,
    pub model_id: String,
    pub context_window_tokens: u32,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
