use std::fmt;

use async_trait::async_trait;
use reqwest::{Method, StatusCode};
use secrecy::SecretString;
use serde::{Deserialize, Serialize, de::IgnoredAny};
use tokio_util::sync::CancellationToken;
use zeroize::Zeroizing;

use crate::domain::{ProviderKind, ValidationResult};

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
const RESPONSES_ENDPOINT: &str = "v1/responses";

pub struct OpenAiProvider {
    transport: ProviderTransport,
    registry: ProviderCapabilityRegistry,
}

impl OpenAiProvider {
    pub fn new() -> Result<Self, AiError> {
        Self::build(ProviderTransport::new(ProviderKind::OpenAi)?)
    }

    fn build(transport: ProviderTransport) -> Result<Self, AiError> {
        let registry = ProviderCapabilityRegistry::load_embedded()
            .map_err(|_| AiError::provider_unavailable())?;
        Ok(Self {
            transport,
            registry,
        })
    }

    #[cfg(test)]
    pub(crate) fn new_for_test(origin: &str) -> Result<Self, AiError> {
        Self::build(ProviderTransport::new_for_test(
            ProviderKind::OpenAi,
            origin,
        )?)
    }
}

impl fmt::Debug for OpenAiProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OpenAiProvider")
            .field("transport", &self.transport)
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl AiProvider for OpenAiProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::OpenAi
    }

    async fn validate(
        &self,
        credential: &SecretString,
        model: &str,
    ) -> Result<ValidationResult, AiError> {
        validate_credential(credential)?;
        validate_model_id(model)?;
        let endpoint = format!("{MODEL_ENDPOINT_PREFIX}{}", encode_path_segment(model)?);
        let request = ProviderHttpRequest::empty(Method::GET, endpoint, CredentialHeader::Bearer)?;
        let response = self
            .transport
            .send_bounded(request, credential, CancellationToken::new())
            .await?;
        let model_response: ModelResponse = response.json()?;
        if model_response.object != "model" || model_response.id != model {
            return Err(AiError::provider_unavailable());
        }

        Ok(ValidationResult {
            model: model.to_owned(),
            context_window_tokens: self.registry.resolve_context_window(
                &ProviderKind::OpenAi,
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

        let body = ResponsesRequest {
            model: request.model,
            stream: true,
            instructions: request.system,
            input: request
                .messages
                .into_iter()
                .map(|message| InputMessage {
                    role: match message.role {
                        UnifiedRole::User => InputRole::User,
                        UnifiedRole::Assistant => InputRole::Assistant,
                    },
                    content: message.content,
                })
                .collect(),
            max_output_tokens: request.max_output_tokens,
            text: TextConfig {
                format: TextFormat { kind: "text" },
            },
        };
        let http_request = ProviderHttpRequest::json(
            Method::POST,
            RESPONSES_ENDPOINT,
            CredentialHeader::Bearer,
            &body,
        )?;
        drop(body);

        Ok(self
            .transport
            .send_stream(http_request, credential, cancel)
            .await?
            .decode(OpenAiEventMapper))
    }
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
            encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
        }
    }
    Ok(encoded)
}

#[derive(Deserialize)]
struct ModelResponse {
    id: String,
    object: String,
}

#[derive(Serialize)]
struct ResponsesRequest {
    model: String,
    stream: bool,
    instructions: String,
    input: Vec<InputMessage>,
    max_output_tokens: u32,
    text: TextConfig,
}

#[derive(Serialize)]
struct InputMessage {
    role: InputRole,
    content: String,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum InputRole {
    User,
    Assistant,
}

#[derive(Serialize)]
struct TextConfig {
    format: TextFormat,
}

#[derive(Serialize)]
struct TextFormat {
    #[serde(rename = "type")]
    kind: &'static str,
}

struct OpenAiEventMapper;

impl SseEventMapper for OpenAiEventMapper {
    fn map_event(&mut self, event: &SseEvent) -> Result<Vec<UnifiedStreamEvent>, AiError> {
        match event.event_type() {
            "response.output_text.delta" => {
                let payload: OutputTextDelta = parse_known_json(event)?;
                if payload.kind != event.event_type() {
                    return Err(AiError::malformed_event());
                }
                Ok(vec![UnifiedStreamEvent::TextDelta {
                    text: payload.delta,
                }])
            }
            "response.completed" => {
                let payload: CompletedEvent = parse_known_json(event)?;
                if payload.kind != event.event_type() || payload.response.status != "completed" {
                    return Err(AiError::malformed_event());
                }
                let mut mapped = Vec::with_capacity(2);
                if let Some(usage) = payload.response.usage {
                    mapped.push(UnifiedStreamEvent::Usage {
                        input_tokens: Some(usage.input_tokens),
                        output_tokens: Some(usage.output_tokens),
                    });
                }
                mapped.push(UnifiedStreamEvent::Completed);
                Ok(mapped)
            }
            "response.failed" => {
                let payload: FailedEvent = parse_known_json(event)?;
                if payload.kind != event.event_type() || payload.response.status != "failed" {
                    return Err(AiError::malformed_event());
                }
                Err(map_stream_error_code(&payload.response.error.code))
            }
            "error" => {
                let payload: ErrorEvent = parse_known_json(event)?;
                if payload.kind != event.event_type() {
                    return Err(AiError::malformed_event());
                }
                Err(AiError::from_http(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    event.data().as_bytes(),
                ))
            }
            "response.incomplete" | "response.cancelled" => Err(AiError::provider_unavailable()),
            _ => Ok(Vec::new()),
        }
    }
}

fn map_stream_error_code(code: &str) -> AiError {
    #[derive(Serialize)]
    struct ErrorEnvelope<'a> {
        error: ErrorCode<'a>,
    }

    #[derive(Serialize)]
    struct ErrorCode<'a> {
        code: &'a str,
    }

    let body = serde_json::to_vec(&ErrorEnvelope {
        error: ErrorCode { code },
    })
    .map(Zeroizing::new)
    .unwrap_or_default();
    AiError::from_http(StatusCode::INTERNAL_SERVER_ERROR, body.as_slice())
}

#[derive(Deserialize)]
struct OutputTextDelta {
    #[serde(rename = "type")]
    kind: String,
    delta: String,
}

#[derive(Deserialize)]
struct CompletedEvent {
    #[serde(rename = "type")]
    kind: String,
    response: CompletedResponse,
}

#[derive(Deserialize)]
struct CompletedResponse {
    status: String,
    usage: Option<CompletionUsage>,
}

#[derive(Deserialize)]
struct CompletionUsage {
    input_tokens: u64,
    output_tokens: u64,
}

#[derive(Deserialize)]
struct FailedEvent {
    #[serde(rename = "type")]
    kind: String,
    response: FailedResponse,
}

#[derive(Deserialize)]
struct FailedResponse {
    status: String,
    error: FailedError,
}

#[derive(Deserialize)]
struct FailedError {
    code: String,
    #[serde(rename = "message")]
    _message: IgnoredAny,
}

#[derive(Deserialize)]
struct ErrorEvent {
    #[serde(rename = "type")]
    kind: String,
    #[serde(rename = "code")]
    _code: Option<String>,
    #[serde(rename = "message")]
    _message: IgnoredAny,
    #[serde(rename = "param")]
    _param: Option<IgnoredAny>,
}
