use std::fmt;

use async_trait::async_trait;
use reqwest::Method;
use secrecy::SecretString;
use serde::{Deserialize, Serialize, de::IgnoredAny};
use tokio_util::sync::CancellationToken;

use crate::{
    domain::{
        ProviderKind, StructuredAnalysisOutcome, StructuredPageRequest, UnifiedMessage,
        UnifiedVisionRequest, ValidationResult,
    },
    errors::{AppError, AppResult},
};

use super::super::{
    error::AiError,
    provider::{
        AiProvider, ProviderStream, UnifiedChatRequest, UnifiedRole, UnifiedStreamEvent,
        validate_credential, validate_model_id,
    },
    registry::ProviderCapabilityRegistry,
    stream::{SseEvent, SseEventMapper, parse_known_json},
    transport::{CredentialHeader, ProviderHttpRequest, ProviderTransport},
};

const MODELS_ENDPOINT: &str = "models";
const CHAT_ENDPOINT: &str = "chat/completions";

pub struct DeepSeekProvider {
    transport: ProviderTransport,
    registry: ProviderCapabilityRegistry,
}
impl DeepSeekProvider {
    pub fn new() -> Result<Self, AiError> {
        Self::build(ProviderTransport::new(ProviderKind::DeepSeek)?)
    }
    fn build(transport: ProviderTransport) -> Result<Self, AiError> {
        Ok(Self {
            transport,
            registry: ProviderCapabilityRegistry::load_embedded()
                .map_err(|_| AiError::provider_unavailable())?,
        })
    }
    #[cfg(test)]
    pub(crate) fn new_for_test(origin: &str) -> Result<Self, AiError> {
        Self::build(ProviderTransport::new_for_test(
            ProviderKind::DeepSeek,
            origin,
        )?)
    }
}
impl fmt::Debug for DeepSeekProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DeepSeekProvider")
            .field("transport", &self.transport)
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl AiProvider for DeepSeekProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::DeepSeek
    }
    async fn validate(
        &self,
        credential: &SecretString,
        model: &str,
    ) -> Result<ValidationResult, AiError> {
        validate_credential(credential)?;
        validate_model_id(model)?;
        let response = self
            .transport
            .send_bounded(
                ProviderHttpRequest::empty(Method::GET, MODELS_ENDPOINT, CredentialHeader::Bearer)?,
                credential,
                CancellationToken::new(),
            )
            .await?;
        let list: ModelList = response.json()?;
        if list.object != "list" || !list.data.into_iter().any(|entry| entry.id == model) {
            return Err(AiError::model_not_found());
        }
        Ok(ValidationResult {
            model: model.to_owned(),
            context_window_tokens: self.registry.resolve_context_window(
                &ProviderKind::DeepSeek,
                model,
                None,
            ),
        })
    }
    async fn stream_chat(
        &self,
        credential: &SecretString,
        request: UnifiedChatRequest,
        cancel: CancellationToken,
    ) -> Result<ProviderStream, AiError> {
        validate_credential(credential)?;
        validate_model_id(&request.model)?;
        if request.max_output_tokens == 0 || request.messages.is_empty() {
            return Err(AiError::invalid_input());
        }
        let body = DeepSeekRequest {
            model: request.model,
            messages: request
                .messages
                .into_iter()
                .map(ChatMessage::from)
                .collect(),
            thinking: Thinking { kind: "disabled" },
            max_tokens: request.max_output_tokens,
            stream: true,
            stream_options: StreamOptions {
                include_usage: true,
            },
        };
        let http = ProviderHttpRequest::json(
            Method::POST,
            CHAT_ENDPOINT,
            CredentialHeader::Bearer,
            &body,
        )?;
        Ok(self
            .transport
            .send_stream(http, credential, cancel)
            .await?
            .decode(DeepSeekEventMapper::default()))
    }

    async fn stream_vision(
        &self,
        _credential: &SecretString,
        _request: UnifiedVisionRequest,
        _cancel: CancellationToken,
    ) -> AppResult<ProviderStream> {
        Err(AppError::unsupported_provider_capability())
    }

    async fn analyze_pages(
        &self,
        _credential: &SecretString,
        _request: StructuredPageRequest,
        _cancel: CancellationToken,
    ) -> AppResult<StructuredAnalysisOutcome> {
        Err(AppError::unsupported_provider_capability())
    }
}
#[derive(Deserialize)]
struct ModelList {
    object: String,
    data: Vec<Model>,
}
#[derive(Deserialize)]
struct Model {
    id: String,
}
#[derive(Serialize)]
struct DeepSeekRequest {
    model: String,
    messages: Vec<ChatMessage>,
    thinking: Thinking,
    max_tokens: u32,
    stream: bool,
    stream_options: StreamOptions,
}
#[derive(Serialize)]
struct Thinking {
    #[serde(rename = "type")]
    kind: &'static str,
}
#[derive(Serialize)]
struct StreamOptions {
    include_usage: bool,
}
#[derive(Serialize)]
struct ChatMessage {
    role: ChatRole,
    content: String,
}
impl From<UnifiedMessage> for ChatMessage {
    fn from(value: UnifiedMessage) -> Self {
        Self {
            role: match value.role {
                UnifiedRole::User => ChatRole::User,
                UnifiedRole::Assistant => ChatRole::Assistant,
            },
            content: value.content,
        }
    }
}
#[derive(Serialize)]
#[serde(rename_all = "lowercase")]
enum ChatRole {
    User,
    Assistant,
}

#[derive(Default)]
struct DeepSeekEventMapper {
    saw_finish: bool,
    saw_text: bool,
}
impl SseEventMapper for DeepSeekEventMapper {
    fn map_event(&mut self, event: &SseEvent) -> Result<Vec<UnifiedStreamEvent>, AiError> {
        if event.event_type() == "error" {
            return Err(AiError::provider_unavailable());
        }
        if event.data() == "[DONE]" {
            return if self.saw_finish && self.saw_text {
                Ok(vec![UnifiedStreamEvent::Completed])
            } else {
                Err(AiError::provider_unavailable())
            };
        }
        let chunk: StreamChunk = parse_known_json(event)?;
        if chunk.error.is_some() {
            return Err(AiError::provider_unavailable());
        }
        match chunk.choices {
            Some(choices) if choices.is_empty() => {
                if !self.saw_finish || chunk.usage.is_none() {
                    return Err(AiError::malformed_event());
                }
                Ok(usage_event(chunk.usage))
            }
            Some(mut choices) => {
                if self.saw_finish || choices.len() != 1 {
                    return Err(AiError::malformed_event());
                }
                let choice = choices.pop().expect("one choice");
                if choice.index != 0 {
                    return Err(AiError::malformed_event());
                }
                let mut output = Vec::new();
                if let Some(content) = choice.delta.content.filter(|content| !content.is_empty()) {
                    self.saw_text = true;
                    output.push(UnifiedStreamEvent::TextDelta { text: content });
                }
                if let Some(finish) = choice.finish_reason {
                    if finish != "stop" {
                        return Err(AiError::provider_unavailable());
                    }
                    self.saw_finish = true;
                }
                if let Some(usage) = chunk.usage {
                    output.extend(usage_event(Some(usage)));
                }
                Ok(output)
            }
            None if chunk.usage.is_some() => Err(AiError::malformed_event()),
            None => Ok(Vec::new()),
        }
    }
}
fn usage_event(usage: Option<Usage>) -> Vec<UnifiedStreamEvent> {
    usage
        .map(|usage| {
            vec![UnifiedStreamEvent::Usage {
                input_tokens: usage.prompt_tokens,
                output_tokens: usage.completion_tokens,
            }]
        })
        .unwrap_or_default()
}
#[derive(Deserialize)]
struct StreamChunk {
    #[serde(default)]
    choices: Option<Vec<Choice>>,
    #[serde(default)]
    usage: Option<Usage>,
    #[serde(default)]
    error: Option<IgnoredAny>,
}
#[derive(Deserialize)]
struct Choice {
    index: u32,
    delta: Delta,
    #[serde(default)]
    finish_reason: Option<String>,
}
#[derive(Deserialize)]
struct Delta {
    #[serde(default)]
    content: Option<String>,
    #[serde(default, rename = "reasoning_content")]
    _reasoning_content: Option<IgnoredAny>,
    #[serde(default, rename = "role")]
    _role: Option<IgnoredAny>,
}
#[derive(Deserialize)]
struct Usage {
    #[serde(default)]
    prompt_tokens: Option<u64>,
    #[serde(default)]
    completion_tokens: Option<u64>,
}
