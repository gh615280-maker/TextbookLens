use std::fmt;

use async_trait::async_trait;
use reqwest::{
    Method,
    header::{HeaderName, HeaderValue},
};
use secrecy::SecretString;
use serde::{Deserialize, Serialize, Serializer, de::IgnoredAny};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use zeroize::Zeroizing;

use crate::{
    domain::{
        AiOperation, CapabilitySupport, ImageLimits, ImageMime, ProviderKind,
        StructuredAnalysisOutcome, StructuredPageRequest, UnifiedMessage, UnifiedVisionRequest,
        ValidationResult,
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
    structured::{
        decode_provider_page_analysis, validate_provider_page_batch, validate_structured_request,
    },
    transport::{CredentialHeader, ProviderHttpRequest, ProviderTransport},
};

const MODEL_ENDPOINT_PREFIX: &str = "v1/models/";
const MESSAGES_ENDPOINT: &str = "v1/messages";
const ANTHROPIC_VERSION: &str = "2023-06-01";
const VERIFIED_VISUAL_MODEL: &str = "claude-sonnet-5";
const STRUCTURED_MAX_OUTPUT_TOKENS: u32 = 4_096;
const STRUCTURED_INSTRUCTION: &str = "Analyze the supplied textbook page images in their input order and return only the requested JSON object.";

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

        let body = vision_messages_request(request)?;
        let http = versioned(
            ProviderHttpRequest::json(
                Method::POST,
                MESSAGES_ENDPOINT,
                CredentialHeader::XApiKey,
                &body,
            )
            .map_err(AiError::into_app_error)?,
        )
        .map_err(AiError::into_app_error)?;
        drop(body);
        let response = self
            .transport
            .send_stream(http, credential, cancel)
            .await
            .map_err(AiError::into_app_error)?;
        Ok(response.decode(AnthropicEventMapper::default()))
    }

    async fn analyze_pages(
        &self,
        credential: &SecretString,
        request: StructuredPageRequest,
        cancel: CancellationToken,
    ) -> AppResult<StructuredAnalysisOutcome> {
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
        let expected_page_count = request.pages.len();
        let body = structured_messages_request(request);
        let http = versioned(
            ProviderHttpRequest::json(
                Method::POST,
                MESSAGES_ENDPOINT,
                CredentialHeader::XApiKey,
                &body,
            )
            .map_err(AiError::into_app_error)?,
        )
        .map_err(AiError::into_app_error)?;
        drop(body);
        let response = self
            .transport
            .send_bounded(http, credential, cancel.clone())
            .await
            .map_err(AiError::into_app_error)?;
        let response: AnthropicStructuredResponse =
            response.json().map_err(AiError::into_app_error)?;
        let visible = response.visible_json()?;
        if cancel.is_cancelled() {
            return Err(cancelled());
        }
        let analysis =
            decode_provider_page_analysis(visible.as_bytes(), &schema_version, max_output_bytes)?;
        validate_provider_page_batch(&analysis, expected_page_count)?;
        if cancel.is_cancelled() {
            return Err(cancelled());
        }
        Ok(StructuredAnalysisOutcome::inline(analysis))
    }
}

impl AnthropicProvider {
    fn verified_limits(&self, model: &str, operation: AiOperation) -> AppResult<ImageLimits> {
        if model != VERIFIED_VISUAL_MODEL
            || self
                .registry
                .operation_support(&ProviderKind::Anthropic, model, operation)
                != CapabilitySupport::Supported
        {
            return Err(AppError::unsupported_provider_capability());
        }
        self.registry
            .capabilities()
            .iter()
            .find(|provider| provider.kind == ProviderKind::Anthropic)
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

fn vision_messages_request(request: UnifiedVisionRequest) -> AppResult<MultimodalMessagesRequest> {
    let UnifiedVisionRequest { text, images } = request;
    let mut messages = normalize_history(text.messages).map_err(AiError::into_app_error)?;
    let last = messages.pop().ok_or_else(invalid_input)?;
    if last.role != AnthropicRole::User || last.content.trim().is_empty() {
        return Err(invalid_input());
    }
    let mut output = messages
        .into_iter()
        .map(|message| MultimodalMessage {
            role: message.role,
            content: MultimodalContent::Text(message.content),
        })
        .collect::<Vec<_>>();
    let mut blocks = images
        .into_iter()
        .map(|image| AnthropicContentBlock::Image {
            source: AnthropicImageSource {
                kind: "base64",
                media_type: image_mime(image.meta.mime_type),
                data: SensitiveString::base64(image.bytes()),
            },
        })
        .collect::<Vec<_>>();
    blocks.push(AnthropicContentBlock::Text { text: last.content });
    output.push(MultimodalMessage {
        role: AnthropicRole::User,
        content: MultimodalContent::Blocks(blocks),
    });
    Ok(MultimodalMessagesRequest {
        model: text.model,
        system: (!text.system.is_empty()).then_some(text.system),
        messages: output,
        max_tokens: text.max_output_tokens,
        stream: true,
        output_config: None,
    })
}

fn structured_messages_request(request: StructuredPageRequest) -> MultimodalMessagesRequest {
    let schema_version = request.schema_version.clone();
    let mut blocks = request
        .pages
        .into_iter()
        .map(|page| AnthropicContentBlock::Image {
            source: AnthropicImageSource {
                kind: "base64",
                media_type: image_mime(page.meta.mime_type),
                data: SensitiveString::base64(page.bytes()),
            },
        })
        .collect::<Vec<_>>();
    blocks.push(AnthropicContentBlock::Text {
        text: format!("{STRUCTURED_INSTRUCTION} Return schema version {schema_version}."),
    });
    MultimodalMessagesRequest {
        model: request.model,
        system: None,
        messages: vec![MultimodalMessage {
            role: AnthropicRole::User,
            content: MultimodalContent::Blocks(blocks),
        }],
        max_tokens: STRUCTURED_MAX_OUTPUT_TOKENS,
        stream: false,
        output_config: Some(AnthropicOutputConfig {
            format: AnthropicJsonFormat {
                kind: "json_schema",
                schema: page_analysis_schema(&schema_version),
            },
        }),
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
struct MultimodalMessagesRequest {
    model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    messages: Vec<MultimodalMessage>,
    max_tokens: u32,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    output_config: Option<AnthropicOutputConfig>,
}

#[derive(Serialize)]
struct MultimodalMessage {
    role: AnthropicRole,
    content: MultimodalContent,
}

#[derive(Serialize)]
#[serde(untagged)]
enum MultimodalContent {
    Text(String),
    Blocks(Vec<AnthropicContentBlock>),
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum AnthropicContentBlock {
    Image { source: AnthropicImageSource },
    Text { text: String },
}

#[derive(Serialize)]
struct AnthropicImageSource {
    #[serde(rename = "type")]
    kind: &'static str,
    media_type: &'static str,
    data: SensitiveString,
}

#[derive(Serialize)]
struct AnthropicOutputConfig {
    format: AnthropicJsonFormat,
}

#[derive(Serialize)]
struct AnthropicJsonFormat {
    #[serde(rename = "type")]
    kind: &'static str,
    schema: Value,
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
        let Some(stop_reason) = value.delta.stop_reason else {
            return Ok(Vec::new());
        };
        if !matches!(stop_reason.as_str(), "end_turn" | "stop_sequence") {
            return Err(AiError::provider_unavailable());
        }
        self.saw_terminal_delta = true;
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

struct SensitiveString(Zeroizing<String>);

impl SensitiveString {
    fn base64(bytes: &[u8]) -> Self {
        let mut encoded = Zeroizing::new(String::with_capacity(bytes.len().div_ceil(3) * 4));
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

const fn image_mime(mime_type: ImageMime) -> &'static str {
    match mime_type {
        ImageMime::Png => "image/png",
        ImageMime::Jpeg => "image/jpeg",
        ImageMime::Webp => "image/webp",
    }
}

#[derive(Deserialize)]
struct AnthropicStructuredResponse {
    #[serde(rename = "type")]
    kind: String,
    role: String,
    content: Vec<AnthropicResponseBlock>,
    stop_reason: Option<String>,
    #[serde(default, rename = "usage")]
    _usage: Option<IgnoredAny>,
}

impl AnthropicStructuredResponse {
    fn visible_json(self) -> AppResult<String> {
        if self.kind != "message" || self.role != "assistant" {
            return Err(AiError::provider_unavailable().into_app_error());
        }
        match self.stop_reason.as_deref() {
            Some("end_turn" | "stop_sequence") => {}
            Some("refusal") => return Err(AiError::refused().into_app_error()),
            _ => return Err(AiError::provider_unavailable().into_app_error()),
        }
        let visible = self
            .content
            .into_iter()
            .filter(|block| block.kind == "text")
            .filter_map(|block| block.text)
            .collect::<String>();
        if visible.is_empty() {
            Err(AiError::provider_unavailable().into_app_error())
        } else {
            Ok(visible)
        }
    }
}

#[derive(Deserialize)]
struct AnthropicResponseBlock {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    text: Option<String>,
    #[serde(default, rename = "thinking")]
    _thinking: Option<IgnoredAny>,
    #[serde(default, rename = "signature")]
    _signature: Option<IgnoredAny>,
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
