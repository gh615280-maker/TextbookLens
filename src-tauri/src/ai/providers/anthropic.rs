use std::fmt;

use async_trait::async_trait;
use reqwest::{
    Method,
    header::{HeaderName, HeaderValue},
};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::domain::{ProviderKind, UnifiedMessage, ValidationResult};

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

const MODEL_ENDPOINT_PREFIX: &str = "v1/models/";
const MESSAGES_ENDPOINT: &str = "v1/messages";
const ANTHROPIC_VERSION: &str = "2023-06-01";

pub struct AnthropicProvider {
    transport: ProviderTransport,
    registry: ProviderCapabilityRegistry,
}

impl AnthropicProvider {
    pub fn new() -> Result<Self, AiError> {
        Self::build(ProviderTransport::new(ProviderKind::Anthropic)?)
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
            ProviderKind::Anthropic,
            origin,
        )?)
    }
}
impl fmt::Debug for AnthropicProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AnthropicProvider")
            .field("transport", &self.transport)
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl AiProvider for AnthropicProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Anthropic
    }
    async fn validate(
        &self,
        credential: &SecretString,
        model: &str,
    ) -> Result<ValidationResult, AiError> {
        validate_credential(credential)?;
        validate_model_id(model)?;
        let endpoint = format!("{MODEL_ENDPOINT_PREFIX}{}", encode_path_segment(model)?);
        let request = versioned(ProviderHttpRequest::empty(
            Method::GET,
            endpoint,
            CredentialHeader::XApiKey,
        )?)?;
        let response = self
            .transport
            .send_bounded(request, credential, CancellationToken::new())
            .await?;
        let metadata: ModelMetadata = response.json()?;
        if metadata.id != model {
            return Err(AiError::provider_unavailable());
        }
        Ok(ValidationResult {
            model: model.to_owned(),
            context_window_tokens: self.registry.resolve_context_window(
                &ProviderKind::Anthropic,
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
        if request.max_output_tokens == 0 {
            return Err(AiError::invalid_input());
        }
        let messages = normalize_history(request.messages)?;
        let body = MessagesRequest {
            model: request.model,
            system: (!request.system.is_empty()).then_some(request.system),
            messages,
            max_tokens: request.max_output_tokens,
            stream: true,
        };
        let http = versioned(ProviderHttpRequest::json(
            Method::POST,
            MESSAGES_ENDPOINT,
            CredentialHeader::XApiKey,
            &body,
        )?)?;
        Ok(self
            .transport
            .send_stream(http, credential, cancel)
            .await?
            .decode(AnthropicEventMapper::default()))
    }
}

fn versioned(request: ProviderHttpRequest) -> Result<ProviderHttpRequest, AiError> {
    request.with_header(
        HeaderName::from_static("anthropic-version"),
        HeaderValue::from_static(ANTHROPIC_VERSION),
    )
}
fn encode_path_segment(value: &str) -> Result<String, AiError> {
    if matches!(value, "." | "..") {
        return Err(AiError::invalid_input());
    }
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(char::from(byte));
        } else {
            encoded.push('%');
            encoded.push(char::from(HEX[usize::from(byte >> 4)]));
            encoded.push(char::from(HEX[usize::from(byte & 15)]));
        }
    }
    Ok(encoded)
}
fn normalize_history(messages: Vec<UnifiedMessage>) -> Result<Vec<AnthropicMessage>, AiError> {
    let mut output = Vec::<AnthropicMessage>::new();
    for message in messages {
        if message.content.trim().is_empty() {
            return Err(AiError::invalid_input());
        }
        let role = match message.role {
            UnifiedRole::User => AnthropicRole::User,
            UnifiedRole::Assistant => AnthropicRole::Assistant,
        };
        if let Some(previous) = output.last_mut()
            && previous.role == role
        {
            previous.content.push('\n');
            previous.content.push_str(&message.content);
        } else {
            output.push(AnthropicMessage {
                role,
                content: message.content,
            });
        }
    }
    if output
        .first()
        .is_none_or(|message| message.role != AnthropicRole::User)
    {
        return Err(AiError::invalid_input());
    }
    Ok(output)
}
#[derive(Deserialize)]
struct ModelMetadata {
    id: String,
}
#[derive(Serialize)]
struct MessagesRequest {
    model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    messages: Vec<AnthropicMessage>,
    max_tokens: u32,
    stream: bool,
}
#[derive(Serialize)]
struct AnthropicMessage {
    role: AnthropicRole,
    content: String,
}
#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
enum AnthropicRole {
    User,
    Assistant,
}

#[derive(Default)]
struct AnthropicEventMapper {
    message_started: bool,
    block: Option<Block>,
    saw_message_delta: bool,
    saw_terminal_delta: bool,
    saw_text: bool,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Block {
    Text,
    Hidden,
}
impl SseEventMapper for AnthropicEventMapper {
    fn map_event(&mut self, event: &SseEvent) -> Result<Vec<UnifiedStreamEvent>, AiError> {
        match event.event_type() {
            "ping" => Ok(Vec::new()),
            "error" => Err(AiError::provider_unavailable()),
            "message_start" => self.start_message(event),
            "content_block_start" => self.start_block(event),
            "content_block_delta" => self.delta(event),
            "content_block_stop" => self.stop_block(event),
            "message_delta" => self.message_delta(event),
            "message_stop" => self.stop_message(event),
            _ => Ok(Vec::new()),
        }
    }
}
impl AnthropicEventMapper {
    fn start_message(&mut self, event: &SseEvent) -> Result<Vec<UnifiedStreamEvent>, AiError> {
        let value: MessageStart = parse_known_json(event)?;
        if self.message_started || value.message.kind != "message" {
            return Err(AiError::malformed_event());
        }
        self.message_started = true;
        Ok(vec![UnifiedStreamEvent::Usage {
            input_tokens: value.message.usage.input_tokens,
            output_tokens: None,
        }])
    }
    fn start_block(&mut self, event: &SseEvent) -> Result<Vec<UnifiedStreamEvent>, AiError> {
        let value: BlockStart = parse_known_json(event)?;
        if !self.message_started || self.block.is_some() || self.saw_message_delta {
            return Err(AiError::malformed_event());
        }
        self.block = Some(if value.content_block.kind == "text" {
            Block::Text
        } else {
            Block::Hidden
        });
        Ok(Vec::new())
    }
    fn delta(&mut self, event: &SseEvent) -> Result<Vec<UnifiedStreamEvent>, AiError> {
        let value: BlockDelta = parse_known_json(event)?;
        let Some(block) = self.block else {
            return Err(AiError::malformed_event());
        };
        if block == Block::Text && value.delta.kind == "text_delta" {
            if value.delta.text.is_empty() {
                return Ok(Vec::new());
            }
            self.saw_text = true;
            return Ok(vec![UnifiedStreamEvent::TextDelta {
                text: value.delta.text,
            }]);
        }
        if block == Block::Hidden
            && matches!(
                value.delta.kind.as_str(),
                "thinking_delta" | "signature_delta" | "input_json_delta"
            )
        {
            return Ok(Vec::new());
        }
        Err(AiError::malformed_event())
    }
    fn stop_block(&mut self, event: &SseEvent) -> Result<Vec<UnifiedStreamEvent>, AiError> {
        let _: Empty = parse_known_json(event)?;
        if self.block.take().is_none() {
            Err(AiError::malformed_event())
        } else {
            Ok(Vec::new())
        }
    }
    fn message_delta(&mut self, event: &SseEvent) -> Result<Vec<UnifiedStreamEvent>, AiError> {
        let value: MessageDelta = parse_known_json(event)?;
        if !self.message_started || self.block.is_some() || self.saw_terminal_delta {
            return Err(AiError::malformed_event());
        }
        self.saw_message_delta = true;
        self.saw_terminal_delta = value.delta.stop_reason.is_some();
        Ok(vec![UnifiedStreamEvent::Usage {
            input_tokens: None,
            output_tokens: value.usage.output_tokens,
        }])
    }
    fn stop_message(&mut self, event: &SseEvent) -> Result<Vec<UnifiedStreamEvent>, AiError> {
        let _: Empty = parse_known_json(event)?;
        if !self.message_started
            || self.block.is_some()
            || !self.saw_message_delta
            || !self.saw_terminal_delta
            || !self.saw_text
        {
            Err(AiError::provider_unavailable())
        } else {
            Ok(vec![UnifiedStreamEvent::Completed])
        }
    }
}
#[derive(Deserialize)]
struct MessageStart {
    message: StartedMessage,
}
#[derive(Deserialize)]
struct StartedMessage {
    #[serde(rename = "type")]
    kind: String,
    usage: InputUsage,
}
#[derive(Deserialize)]
struct InputUsage {
    input_tokens: Option<u64>,
}
#[derive(Deserialize)]
struct BlockStart {
    content_block: ContentBlock,
}
#[derive(Deserialize)]
struct ContentBlock {
    #[serde(rename = "type")]
    kind: String,
}
#[derive(Deserialize)]
struct BlockDelta {
    delta: Delta,
}
#[derive(Deserialize)]
struct Delta {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    text: String,
}
#[derive(Deserialize)]
struct MessageDelta {
    delta: StopDelta,
    usage: OutputUsage,
}
#[derive(Deserialize)]
struct StopDelta {
    stop_reason: Option<String>,
}
#[derive(Deserialize)]
struct OutputUsage {
    output_tokens: Option<u64>,
}
#[derive(Deserialize)]
struct Empty {}
