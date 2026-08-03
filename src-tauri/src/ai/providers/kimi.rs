use std::fmt;

use async_trait::async_trait;
use reqwest::Method;
use secrecy::SecretString;
use serde::{Deserialize, Serialize, Serializer, de::IgnoredAny};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use zeroize::Zeroizing;

use crate::{
    domain::{
        AiOperation, CapabilitySupport, ImageLimits, ImageMime, ProviderKind, ProviderPageAnalysis,
        StructuredPageRequest, UnifiedMessage, UnifiedVisionRequest, ValidationResult,
    },
    errors::{AppError, AppErrorCode, AppResult},
};

use super::super::{
    error::AiError,
    multimodal::validate_vision_request,
    provider::{
        AiProvider, ProviderStream, UnifiedChatRequest, UnifiedRole, UnifiedStreamEvent,
        validate_credential, validate_model_id,
    },
    registry::ProviderCapabilityRegistry,
    stream::{SseEvent, SseEventMapper, parse_known_json},
    structured::{decode_provider_page_analysis, validate_structured_request},
    transport::{CredentialHeader, ProviderHttpRequest, ProviderTransport},
};

const MODELS_ENDPOINT: &str = "models";
const CHAT_ENDPOINT: &str = "chat/completions";
const VERIFIED_VISUAL_MODEL: &str = "kimi-k3";
const STRUCTURED_MAX_OUTPUT_TOKENS: u32 = 4_096;
const STRUCTURED_INSTRUCTION: &str = "Analyze the supplied textbook page images in their input order and return only the requested JSON object.";

pub struct KimiProvider {
    transport: ProviderTransport,
    registry: ProviderCapabilityRegistry,
}
impl KimiProvider {
    pub fn new() -> Result<Self, AiError> {
        Self::build(ProviderTransport::new(ProviderKind::Kimi)?)
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
        Self::build(ProviderTransport::new_for_test(ProviderKind::Kimi, origin)?)
    }
}
impl fmt::Debug for KimiProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KimiProvider")
            .field("transport", &self.transport)
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl AiProvider for KimiProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Kimi
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
                &ProviderKind::Kimi,
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
        let mut messages = Vec::with_capacity(request.messages.len() + 1);
        if !request.system.is_empty() {
            messages.push(ChatMessage {
                role: ChatRole::System,
                content: request.system,
            });
        }
        messages.extend(request.messages.into_iter().map(ChatMessage::from));
        let body = KimiRequest {
            model: request.model,
            messages,
            max_completion_tokens: request.max_output_tokens,
            stream: true,
            stream_options: StreamOptions {
                include_usage: true,
            },
            reasoning_effort: "low",
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
            .decode(KimiEventMapper::default()))
    }

    async fn stream_vision(
        &self,
        credential: &SecretString,
        request: UnifiedVisionRequest,
        cancel: CancellationToken,
    ) -> AppResult<ProviderStream> {
        let limits = self.verified_limits(&request.text.model, AiOperation::VisionLearning)?;
        validate_vision_input(&request, limits)?;
        if cancel.is_cancelled() {
            return Err(cancelled());
        }

        let body = vision_chat_request(request)?;
        let http =
            ProviderHttpRequest::json(Method::POST, CHAT_ENDPOINT, CredentialHeader::Bearer, &body)
                .map_err(AiError::into_app_error)?;
        drop(body);
        let response = self
            .transport
            .send_stream(http, credential, cancel)
            .await
            .map_err(AiError::into_app_error)?;
        Ok(response.decode(KimiEventMapper::default()))
    }

    async fn analyze_pages(
        &self,
        credential: &SecretString,
        request: StructuredPageRequest,
        cancel: CancellationToken,
    ) -> AppResult<ProviderPageAnalysis> {
        let limits = self.verified_limits(&request.model, AiOperation::StructuredPageAnalysis)?;
        let book_id = request
            .pages
            .first()
            .map(|page| page.meta.book_id)
            .ok_or_else(invalid_input)?;
        validate_structured_request(book_id, &request, limits)?;
        if cancel.is_cancelled() {
            return Err(cancelled());
        }

        let schema_version = request.schema_version.clone();
        let max_output_bytes = request.max_output_bytes;
        let body = structured_chat_request(request);
        let http =
            ProviderHttpRequest::json(Method::POST, CHAT_ENDPOINT, CredentialHeader::Bearer, &body)
                .map_err(AiError::into_app_error)?;
        drop(body);
        let response = self
            .transport
            .send_bounded(http, credential, cancel.clone())
            .await
            .map_err(AiError::into_app_error)?;
        let response: KimiStructuredResponse = response.json().map_err(AiError::into_app_error)?;
        let visible = response.visible_json()?;
        if cancel.is_cancelled() {
            return Err(cancelled());
        }
        decode_provider_page_analysis(visible.as_bytes(), &schema_version, max_output_bytes)
    }
}

impl KimiProvider {
    fn verified_limits(&self, model: &str, operation: AiOperation) -> AppResult<ImageLimits> {
        if model != VERIFIED_VISUAL_MODEL
            || self
                .registry
                .operation_support(&ProviderKind::Kimi, model, operation)
                != CapabilitySupport::Supported
        {
            return Err(AppError::unsupported_provider_capability());
        }
        self.registry
            .capabilities()
            .iter()
            .find(|provider| provider.kind == ProviderKind::Kimi)
            .and_then(|provider| provider.models.iter().find(|entry| entry.id == model))
            .and_then(|entry| entry.image_limits)
            .ok_or_else(AppError::unsupported_provider_capability)
    }
}

fn validate_vision_input(request: &UnifiedVisionRequest, limits: ImageLimits) -> AppResult<()> {
    validate_model_id(&request.text.model).map_err(AiError::into_app_error)?;
    if request.text.max_output_tokens == 0
        || request.text.messages.is_empty()
        || request.text.messages.last().map(|message| &message.role) != Some(&UnifiedRole::User)
    {
        return Err(invalid_input());
    }
    let book_id = request
        .images
        .first()
        .map(|image| image.meta.book_id)
        .ok_or_else(invalid_input)?;
    validate_vision_request(book_id, request, limits)
}

fn vision_chat_request(request: UnifiedVisionRequest) -> AppResult<KimiMultimodalRequest> {
    let UnifiedVisionRequest { text, images } = request;
    let mut source = text.messages;
    let last = source.pop().ok_or_else(invalid_input)?;
    if last.role != UnifiedRole::User || last.content.trim().is_empty() {
        return Err(invalid_input());
    }
    let mut messages = Vec::with_capacity(source.len() + 2);
    if !text.system.is_empty() {
        messages.push(KimiMultimodalMessage {
            role: ChatRole::System,
            content: KimiMultimodalContent::Text(text.system),
        });
    }
    messages.extend(source.into_iter().map(|message| KimiMultimodalMessage {
        role: match message.role {
            UnifiedRole::User => ChatRole::User,
            UnifiedRole::Assistant => ChatRole::Assistant,
        },
        content: KimiMultimodalContent::Text(message.content),
    }));
    let mut parts = images
        .into_iter()
        .map(|image| KimiContentPart::ImageUrl {
            image_url: KimiImageUrl {
                url: SensitiveString::data_url(image.meta.mime_type, image.bytes()),
            },
        })
        .collect::<Vec<_>>();
    parts.push(KimiContentPart::Text { text: last.content });
    messages.push(KimiMultimodalMessage {
        role: ChatRole::User,
        content: KimiMultimodalContent::Parts(parts),
    });
    Ok(KimiMultimodalRequest {
        model: text.model,
        messages,
        max_completion_tokens: text.max_output_tokens,
        stream: true,
        stream_options: Some(StreamOptions {
            include_usage: true,
        }),
        reasoning_effort: "low",
        response_format: None,
    })
}

fn structured_chat_request(request: StructuredPageRequest) -> KimiMultimodalRequest {
    let schema_version = request.schema_version.clone();
    let mut parts = request
        .pages
        .into_iter()
        .map(|page| KimiContentPart::ImageUrl {
            image_url: KimiImageUrl {
                url: SensitiveString::data_url(page.meta.mime_type, page.bytes()),
            },
        })
        .collect::<Vec<_>>();
    parts.push(KimiContentPart::Text {
        text: format!("{STRUCTURED_INSTRUCTION} Return schema version {schema_version}."),
    });
    KimiMultimodalRequest {
        model: request.model,
        messages: vec![KimiMultimodalMessage {
            role: ChatRole::User,
            content: KimiMultimodalContent::Parts(parts),
        }],
        max_completion_tokens: STRUCTURED_MAX_OUTPUT_TOKENS,
        stream: false,
        stream_options: None,
        reasoning_effort: "low",
        response_format: Some(KimiResponseFormat {
            kind: "json_schema",
            json_schema: KimiJsonSchema {
                name: "textbooklens_page_analysis",
                strict: true,
                schema: page_analysis_schema(&schema_version),
            },
        }),
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
struct KimiRequest {
    model: String,
    messages: Vec<ChatMessage>,
    max_completion_tokens: u32,
    stream: bool,
    stream_options: StreamOptions,
    reasoning_effort: &'static str,
}

#[derive(Serialize)]
struct KimiMultimodalRequest {
    model: String,
    messages: Vec<KimiMultimodalMessage>,
    max_completion_tokens: u32,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream_options: Option<StreamOptions>,
    reasoning_effort: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<KimiResponseFormat>,
}

#[derive(Serialize)]
struct KimiMultimodalMessage {
    role: ChatRole,
    content: KimiMultimodalContent,
}

#[derive(Serialize)]
#[serde(untagged)]
enum KimiMultimodalContent {
    Text(String),
    Parts(Vec<KimiContentPart>),
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum KimiContentPart {
    ImageUrl { image_url: KimiImageUrl },
    Text { text: String },
}

#[derive(Serialize)]
struct KimiImageUrl {
    url: SensitiveString,
}

#[derive(Serialize)]
struct KimiResponseFormat {
    #[serde(rename = "type")]
    kind: &'static str,
    json_schema: KimiJsonSchema,
}

#[derive(Serialize)]
struct KimiJsonSchema {
    name: &'static str,
    strict: bool,
    schema: Value,
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
    System,
    User,
    Assistant,
}

#[derive(Default)]
struct KimiEventMapper {
    saw_finish: bool,
    saw_text: bool,
}
impl SseEventMapper for KimiEventMapper {
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
                    Err(AiError::malformed_event())
                } else {
                    Ok(usage_event(chunk.usage))
                }
            }
            Some(choices) => {
                if self.saw_finish {
                    return Err(AiError::malformed_event());
                }
                let choice = choices
                    .into_iter()
                    .next()
                    .ok_or_else(AiError::malformed_event)?;
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

struct SensitiveString(Zeroizing<String>);

impl SensitiveString {
    fn data_url(mime_type: ImageMime, bytes: &[u8]) -> Self {
        let prefix = match mime_type {
            ImageMime::Png => "data:image/png;base64,",
            ImageMime::Jpeg => "data:image/jpeg;base64,",
            ImageMime::Webp => "data:image/webp;base64,",
        };
        let mut encoded = Zeroizing::new(String::with_capacity(
            prefix.len() + bytes.len().div_ceil(3) * 4,
        ));
        encoded.push_str(prefix);
        encode_base64(bytes, &mut encoded);
        Self(encoded)
    }
}

impl Serialize for SensitiveString {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.0.as_str())
    }
}

fn encode_base64(bytes: &[u8], output: &mut String) {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    for chunk in bytes.chunks(3) {
        let first = chunk[0];
        let second = chunk.get(1).copied().unwrap_or(0);
        let third = chunk.get(2).copied().unwrap_or(0);
        output.push(char::from(TABLE[usize::from(first >> 2)]));
        output.push(char::from(
            TABLE[usize::from(((first & 0x03) << 4) | (second >> 4))],
        ));
        output.push(if chunk.len() > 1 {
            char::from(TABLE[usize::from(((second & 0x0f) << 2) | (third >> 6))])
        } else {
            '='
        });
        output.push(if chunk.len() > 2 {
            char::from(TABLE[usize::from(third & 0x3f)])
        } else {
            '='
        });
    }
}

#[derive(Deserialize)]
struct KimiStructuredResponse {
    choices: Vec<KimiStructuredChoice>,
    #[serde(default, rename = "usage")]
    _usage: Option<IgnoredAny>,
}

impl KimiStructuredResponse {
    fn visible_json(mut self) -> AppResult<String> {
        if self.choices.len() != 1 {
            return Err(AiError::provider_unavailable().into_app_error());
        }
        let choice = self.choices.remove(0);
        if choice.index != 0 {
            return Err(AiError::provider_unavailable().into_app_error());
        }
        match choice.finish_reason.as_str() {
            "stop" => {}
            "content_filter" => return Err(AiError::refused().into_app_error()),
            _ => return Err(AiError::provider_unavailable().into_app_error()),
        }
        let visible = choice.message.content.unwrap_or_default();
        if visible.is_empty() {
            Err(AiError::provider_unavailable().into_app_error())
        } else {
            Ok(visible)
        }
    }
}

#[derive(Deserialize)]
struct KimiStructuredChoice {
    index: u32,
    finish_reason: String,
    message: KimiStructuredMessage,
}

#[derive(Deserialize)]
struct KimiStructuredMessage {
    content: Option<String>,
    #[serde(default, rename = "reasoning_content")]
    _reasoning_content: Option<IgnoredAny>,
    #[serde(default, rename = "tool_calls")]
    _tool_calls: Option<IgnoredAny>,
}

fn page_analysis_schema(schema_version: &str) -> Value {
    let nullable_string = || json!({"anyOf":[{"type":"string"},{"type":"null"}]});
    json!({
        "type":"object",
        "properties":{
            "schemaVersion":{"type":"string","enum":[schema_version]},
            "pages":{"type":"array","items":{
                "type":"object","properties":{
                    "pageNumber":{"type":"integer","minimum":0},
                    "blocks":{"type":"array","items":{
                        "type":"object","properties":{
                            "ordinal":{"type":"integer","minimum":0},
                            "kind":{"type":"string","enum":["title","paragraph","list","table","caption","formula","figure","transcript"]},
                            "plainText":{"type":"string"},
                            "bounds":{"anyOf":[{"type":"object","properties":{
                                "x":{"type":"number"},"y":{"type":"number"},"width":{"type":"number"},"height":{"type":"number"}
                            },"required":["x","y","width","height"],"additionalProperties":false},{"type":"null"}]},
                            "latex":nullable_string(),
                            "tableCells":{"anyOf":[{"type":"array","items":{
                                "type":"object","properties":{
                                    "row":{"type":"integer","minimum":0},"column":{"type":"integer","minimum":0},
                                    "rowSpan":{"type":"integer","minimum":1},"columnSpan":{"type":"integer","minimum":1},"text":{"type":"string"}
                                },"required":["row","column","rowSpan","columnSpan","text"],"additionalProperties":false
                            }},{"type":"null"}]},
                            "visualDescription":nullable_string()
                        },
                        "required":["ordinal","kind","plainText","bounds","latex","tableCells","visualDescription"],
                        "additionalProperties":false
                    }}
                },"required":["pageNumber","blocks"],"additionalProperties":false
            }}
        },"required":["schemaVersion","pages"],"additionalProperties":false
    })
}

fn invalid_input() -> AppError {
    AppError::new(AppErrorCode::InvalidInput)
}

fn cancelled() -> AppError {
    AiError::cancelled().into_app_error()
}
