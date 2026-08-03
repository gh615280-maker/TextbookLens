use std::{collections::BTreeMap, fmt};

use async_trait::async_trait;
use reqwest::{Method, StatusCode};
use secrecy::SecretString;
use serde::{Deserialize, Serialize, de::IgnoredAny};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::domain::{ProviderKind, ValidationResult};

use super::super::{
    error::AiError,
    provider::{
        AiProvider, ProviderStream, UnifiedChatRequest, UnifiedRole, UnifiedStreamEvent,
        validate_credential, validate_model_id,
    },
    stream::{SseEvent, SseEventMapper, parse_known_json},
    transport::{CredentialHeader, ProviderHttpRequest, ProviderTransport},
};

const MODEL_ENDPOINT_PREFIX: &str = "v1beta/models/";

pub struct GeminiProvider {
    transport: ProviderTransport,
}

impl GeminiProvider {
    pub fn new() -> Result<Self, AiError> {
        Ok(Self {
            transport: ProviderTransport::new(ProviderKind::Gemini)?,
        })
    }

    #[cfg(test)]
    pub(crate) fn new_for_test(origin: &str) -> Result<Self, AiError> {
        Ok(Self {
            transport: ProviderTransport::new_for_test(ProviderKind::Gemini, origin)?,
        })
    }
}

impl fmt::Debug for GeminiProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GeminiProvider")
            .field("transport", &self.transport)
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl AiProvider for GeminiProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Gemini
    }

    async fn validate(
        &self,
        credential: &SecretString,
        model: &str,
    ) -> Result<ValidationResult, AiError> {
        validate_credential(credential)?;
        validate_model_id(model)?;
        let endpoint = format!("{MODEL_ENDPOINT_PREFIX}{}", encode_path_segment(model)?);
        let response = self
            .transport
            .send_bounded(
                ProviderHttpRequest::empty(Method::GET, endpoint, CredentialHeader::XGoogApiKey)?,
                credential,
                CancellationToken::new(),
            )
            .await?;
        let metadata: ModelMetadata = response.json()?;
        if metadata.name != format!("models/{model}")
            || metadata.input_token_limit == 0
            || metadata.output_token_limit == 0
            || !metadata
                .supported_generation_methods
                .iter()
                .any(|method| method == "generateContent")
        {
            return Err(AiError::provider_unavailable());
        }

        Ok(ValidationResult {
            model: model.to_owned(),
            context_window_tokens: metadata.input_token_limit,
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
        let contents = normalize_history(request.messages)?;
        let body = GeminiRequest {
            system_instruction: (!request.system.is_empty()).then(|| GeminiContent {
                role: None,
                parts: vec![TextPart {
                    text: request.system,
                }],
            }),
            contents,
            generation_config: GenerationConfig {
                max_output_tokens: request.max_output_tokens,
            },
        };
        let endpoint = format!(
            "{MODEL_ENDPOINT_PREFIX}{}:streamGenerateContent?alt=sse",
            encode_path_segment(&request.model)?
        );
        let http_request = ProviderHttpRequest::json(
            Method::POST,
            endpoint,
            CredentialHeader::XGoogApiKey,
            &body,
        )?;
        drop(body);

        Ok(self
            .transport
            .send_stream(http_request, credential, cancel)
            .await?
            .decode(GeminiEventMapper::default()))
    }
}

fn normalize_history(
    messages: Vec<crate::domain::UnifiedMessage>,
) -> Result<Vec<GeminiContent>, AiError> {
    if messages.is_empty() {
        return Err(AiError::invalid_input());
    }
    let mut contents = Vec::<GeminiContent>::new();
    for message in messages {
        if message.content.trim().is_empty() {
            return Err(AiError::invalid_input());
        }
        let role = match message.role {
            UnifiedRole::User => GeminiRole::User,
            UnifiedRole::Assistant => GeminiRole::Model,
        };
        if let Some(previous) = contents.last_mut()
            && previous.role == Some(role)
        {
            previous.parts.push(TextPart {
                text: message.content,
            });
        } else {
            contents.push(GeminiContent {
                role: Some(role),
                parts: vec![TextPart {
                    text: message.content,
                }],
            });
        }
    }
    if contents.first().and_then(|content| content.role) != Some(GeminiRole::User)
        || contents.windows(2).any(|pair| pair[0].role == pair[1].role)
    {
        return Err(AiError::invalid_input());
    }
    Ok(contents)
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
#[serde(rename_all = "camelCase")]
struct ModelMetadata {
    name: String,
    input_token_limit: u32,
    output_token_limit: u32,
    supported_generation_methods: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GeminiRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    system_instruction: Option<GeminiContent>,
    contents: Vec<GeminiContent>,
    generation_config: GenerationConfig,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GeminiContent {
    #[serde(skip_serializing_if = "Option::is_none")]
    role: Option<GeminiRole>,
    parts: Vec<TextPart>,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
enum GeminiRole {
    User,
    Model,
}

#[derive(Serialize)]
struct TextPart {
    text: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GenerationConfig {
    max_output_tokens: u32,
}

#[derive(Default)]
struct GeminiEventMapper {
    saw_visible_text: bool,
}

impl SseEventMapper for GeminiEventMapper {
    fn map_event(&mut self, event: &SseEvent) -> Result<Vec<UnifiedStreamEvent>, AiError> {
        match event.event_type() {
            "error" => Err(AiError::from_provider_http(
                &ProviderKind::Gemini,
                StatusCode::INTERNAL_SERVER_ERROR,
                event.data().as_bytes(),
            )),
            "message" | "" => self.map_response(event),
            _ => Ok(Vec::new()),
        }
    }
}

impl GeminiEventMapper {
    fn map_response(&mut self, event: &SseEvent) -> Result<Vec<UnifiedStreamEvent>, AiError> {
        let response: GeminiResponse = parse_known_json(event)?;
        if response
            .prompt_feedback
            .as_ref()
            .is_some_and(|feedback| feedback.block_reason.is_some())
        {
            return Err(AiError::refused());
        }

        let Some(candidates) = response.candidates else {
            return if response.extra.is_empty() {
                Err(AiError::malformed_event())
            } else {
                Ok(Vec::new())
            };
        };
        let candidate = candidates.first().ok_or_else(AiError::malformed_event)?;
        let candidate: Candidate =
            serde_json::from_value(candidate.clone()).map_err(|_| AiError::malformed_event())?;
        if candidate.index.is_some_and(|index| index != 0) {
            return Err(AiError::malformed_event());
        }
        let content = candidate.content.ok_or_else(AiError::malformed_event)?;
        let parts = content.parts.ok_or_else(AiError::malformed_event)?;
        if parts.is_empty() {
            return Err(AiError::malformed_event());
        }
        if let Some(reason) = candidate.finish_reason.as_deref()
            && !matches!(reason, "STOP" | "MAX_TOKENS")
        {
            return Err(match reason {
                "SAFETY"
                | "RECITATION"
                | "LANGUAGE"
                | "BLOCKLIST"
                | "PROHIBITED_CONTENT"
                | "SPII"
                | "MALFORMED_FUNCTION_CALL"
                | "IMAGE_SAFETY" => AiError::refused(),
                _ => AiError::provider_unavailable(),
            });
        }

        let text = parts
            .into_iter()
            .filter(|part| !part.thought.unwrap_or(false))
            .filter_map(|part| part.text.filter(|text| !text.is_empty()))
            .collect::<Vec<_>>();
        if candidate.finish_reason.is_some() && !self.saw_visible_text && text.is_empty() {
            return Err(AiError::provider_unavailable());
        }
        self.saw_visible_text |= !text.is_empty();

        let mut mapped = text
            .into_iter()
            .map(|text| UnifiedStreamEvent::TextDelta { text })
            .collect::<Vec<_>>();
        if let Some(usage) = response.usage_metadata {
            mapped.push(UnifiedStreamEvent::Usage {
                input_tokens: Some(usage.prompt_token_count),
                output_tokens: Some(usage.candidates_token_count),
            });
        }
        if candidate.finish_reason.is_some() {
            mapped.push(UnifiedStreamEvent::Completed);
        }
        Ok(mapped)
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiResponse {
    candidates: Option<Vec<Value>>,
    prompt_feedback: Option<PromptFeedback>,
    usage_metadata: Option<UsageMetadata>,
    #[serde(flatten)]
    extra: BTreeMap<String, Value>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PromptFeedback {
    block_reason: Option<IgnoredAny>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UsageMetadata {
    prompt_token_count: u64,
    candidates_token_count: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Candidate {
    content: Option<CandidateContent>,
    finish_reason: Option<String>,
    index: Option<u32>,
}

#[derive(Deserialize)]
struct CandidateContent {
    parts: Option<Vec<CandidatePart>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CandidatePart {
    text: Option<String>,
    thought: Option<bool>,
    #[serde(rename = "thoughtSignature")]
    _thought_signature: Option<IgnoredAny>,
}
