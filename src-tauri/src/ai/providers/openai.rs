use std::fmt;

use async_trait::async_trait;
use reqwest::{Method, StatusCode};
use secrecy::SecretString;
use serde::{Deserialize, Serialize, Serializer, de::IgnoredAny};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use zeroize::Zeroizing;

use crate::{
    domain::{
        AiOperation, CapabilitySupport, ImageLimits, ImageMime, ProviderKind, ProviderPageAnalysis,
        StructuredPageRequest, UnifiedVisionRequest, ValidationResult,
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

const MODEL_ENDPOINT_PREFIX: &str = "v1/models/";
const RESPONSES_ENDPOINT: &str = "v1/responses";
const VERIFIED_VISUAL_MODEL: &str = "gpt-5.6";
const STRUCTURED_MAX_OUTPUT_TOKENS: u32 = 4_096;
const STRUCTURED_INSTRUCTIONS: &str = "Analyze the supplied textbook page images in their input order and return only the requested JSON object.";

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
                    content: InputContent::Text(message.content),
                })
                .collect(),
            max_output_tokens: request.max_output_tokens,
            text: TextConfig {
                format: TextFormat::Plain(PlainTextFormat { kind: "text" }),
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
            .decode(OpenAiEventMapper::default()))
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

        let body = vision_responses_request(request, true, None)?;
        let http_request = ProviderHttpRequest::json(
            Method::POST,
            RESPONSES_ENDPOINT,
            CredentialHeader::Bearer,
            &body,
        )
        .map_err(AiError::into_app_error)?;
        drop(body);

        let response = self
            .transport
            .send_stream(http_request, credential, cancel)
            .await
            .map_err(AiError::into_app_error)?;
        Ok(response.decode(OpenAiEventMapper::default()))
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
        let body = structured_responses_request(request);
        let http_request = ProviderHttpRequest::json(
            Method::POST,
            RESPONSES_ENDPOINT,
            CredentialHeader::Bearer,
            &body,
        )
        .map_err(AiError::into_app_error)?;
        drop(body);
        let response = self
            .transport
            .send_bounded(http_request, credential, cancel.clone())
            .await
            .map_err(AiError::into_app_error)?;
        let response: StructuredResponse = response.json().map_err(AiError::into_app_error)?;
        let visible = response.visible_json()?;
        if cancel.is_cancelled() {
            return Err(cancelled());
        }
        decode_provider_page_analysis(visible.as_bytes(), &schema_version, max_output_bytes)
    }
}

impl OpenAiProvider {
    fn verified_limits(&self, model: &str, operation: AiOperation) -> AppResult<ImageLimits> {
        if model != VERIFIED_VISUAL_MODEL
            || self
                .registry
                .operation_support(&ProviderKind::OpenAi, model, operation)
                != CapabilitySupport::Supported
        {
            return Err(AppError::unsupported_provider_capability());
        }
        self.registry
            .capabilities()
            .iter()
            .find(|provider| provider.kind == ProviderKind::OpenAi)
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

fn vision_responses_request(
    request: UnifiedVisionRequest,
    stream: bool,
    structured_schema: Option<Value>,
) -> AppResult<ResponsesRequest> {
    let UnifiedVisionRequest { text, images } = request;
    let mut messages = text.messages;
    let last = messages.pop().ok_or_else(invalid_input)?;
    if last.role != UnifiedRole::User || last.content.trim().is_empty() {
        return Err(invalid_input());
    }
    let mut input = messages
        .into_iter()
        .map(|message| InputMessage {
            role: match message.role {
                UnifiedRole::User => InputRole::User,
                UnifiedRole::Assistant => InputRole::Assistant,
            },
            content: InputContent::Text(message.content),
        })
        .collect::<Vec<_>>();
    let mut parts = images
        .into_iter()
        .map(|image| ResponseContentPart::InputImage {
            image_url: SensitiveString::data_url(image.meta.mime_type, image.bytes()),
            detail: "auto",
        })
        .collect::<Vec<_>>();
    parts.push(ResponseContentPart::InputText { text: last.content });
    input.push(InputMessage {
        role: InputRole::User,
        content: InputContent::Parts(parts),
    });

    let format = structured_schema.map_or_else(
        || TextFormat::Plain(PlainTextFormat { kind: "text" }),
        |schema| {
            TextFormat::JsonSchema(JsonSchemaFormat {
                kind: "json_schema",
                name: "textbooklens_page_analysis",
                strict: true,
                schema,
            })
        },
    );
    Ok(ResponsesRequest {
        model: text.model,
        stream,
        instructions: text.system,
        input,
        max_output_tokens: text.max_output_tokens,
        text: TextConfig { format },
    })
}

fn structured_responses_request(request: StructuredPageRequest) -> ResponsesRequest {
    let schema_version = request.schema_version.clone();
    let mut parts = request
        .pages
        .into_iter()
        .map(|page| ResponseContentPart::InputImage {
            image_url: SensitiveString::data_url(page.meta.mime_type, page.bytes()),
            detail: "auto",
        })
        .collect::<Vec<_>>();
    parts.push(ResponseContentPart::InputText {
        text: format!(
            "Return schema version {schema_version}; the images are ordered page inputs."
        ),
    });
    ResponsesRequest {
        model: request.model,
        stream: false,
        instructions: STRUCTURED_INSTRUCTIONS.to_owned(),
        input: vec![InputMessage {
            role: InputRole::User,
            content: InputContent::Parts(parts),
        }],
        max_output_tokens: STRUCTURED_MAX_OUTPUT_TOKENS,
        text: TextConfig {
            format: TextFormat::JsonSchema(JsonSchemaFormat {
                kind: "json_schema",
                name: "textbooklens_page_analysis",
                strict: true,
                schema: page_analysis_schema(&schema_version),
            }),
        },
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
    content: InputContent,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum InputRole {
    User,
    Assistant,
}

#[derive(Serialize)]
#[serde(untagged)]
enum InputContent {
    Text(String),
    Parts(Vec<ResponseContentPart>),
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ResponseContentPart {
    InputText {
        text: String,
    },
    InputImage {
        image_url: SensitiveString,
        detail: &'static str,
    },
}

#[derive(Serialize)]
struct TextConfig {
    format: TextFormat,
}

#[derive(Serialize)]
#[serde(untagged)]
enum TextFormat {
    Plain(PlainTextFormat),
    JsonSchema(JsonSchemaFormat),
}

#[derive(Serialize)]
struct PlainTextFormat {
    #[serde(rename = "type")]
    kind: &'static str,
}

#[derive(Serialize)]
struct JsonSchemaFormat {
    #[serde(rename = "type")]
    kind: &'static str,
    name: &'static str,
    strict: bool,
    schema: Value,
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
struct StructuredResponse {
    status: String,
    output: Vec<StructuredOutputItem>,
    #[serde(default, rename = "usage")]
    _usage: Option<IgnoredAny>,
}

impl StructuredResponse {
    fn visible_json(self) -> AppResult<String> {
        if self.status != "completed" {
            return Err(AiError::provider_unavailable().into_app_error());
        }
        let mut visible = String::new();
        for item in self.output {
            if item.kind != "message" {
                continue;
            }
            for content in item.content.unwrap_or_default() {
                match content.kind.as_str() {
                    "output_text" => {
                        let text = content
                            .text
                            .ok_or_else(|| AiError::provider_unavailable().into_app_error())?;
                        visible.push_str(&text);
                    }
                    "refusal" => return Err(AiError::refused().into_app_error()),
                    _ => {}
                }
            }
        }
        if visible.is_empty() {
            Err(AiError::provider_unavailable().into_app_error())
        } else {
            Ok(visible)
        }
    }
}

#[derive(Deserialize)]
struct StructuredOutputItem {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    content: Option<Vec<StructuredOutputContent>>,
}

#[derive(Deserialize)]
struct StructuredOutputContent {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    text: Option<String>,
    #[serde(default, rename = "refusal")]
    _refusal: Option<IgnoredAny>,
}

fn page_analysis_schema(schema_version: &str) -> Value {
    let nullable_string = || json!({"anyOf":[{"type":"string"},{"type":"null"}]});
    json!({
        "type":"object",
        "properties":{
            "schemaVersion":{"type":"string","enum":[schema_version]},
            "pages":{
                "type":"array",
                "items":{
                    "type":"object",
                    "properties":{
                        "pageNumber":{"type":"integer","minimum":0},
                        "blocks":{
                            "type":"array",
                            "items":{
                                "type":"object",
                                "properties":{
                                    "ordinal":{"type":"integer","minimum":0},
                                    "kind":{"type":"string","enum":["title","paragraph","list","table","caption","formula","figure","transcript"]},
                                    "plainText":{"type":"string"},
                                    "bounds":{"anyOf":[{
                                        "type":"object",
                                        "properties":{
                                            "x":{"type":"number"},"y":{"type":"number"},
                                            "width":{"type":"number"},"height":{"type":"number"}
                                        },
                                        "required":["x","y","width","height"],
                                        "additionalProperties":false
                                    },{"type":"null"}]},
                                    "latex":nullable_string(),
                                    "tableCells":{"anyOf":[{
                                        "type":"array",
                                        "items":{
                                            "type":"object",
                                            "properties":{
                                                "row":{"type":"integer","minimum":0},
                                                "column":{"type":"integer","minimum":0},
                                                "rowSpan":{"type":"integer","minimum":1},
                                                "columnSpan":{"type":"integer","minimum":1},
                                                "text":{"type":"string"}
                                            },
                                            "required":["row","column","rowSpan","columnSpan","text"],
                                            "additionalProperties":false
                                        }
                                    },{"type":"null"}]},
                                    "visualDescription":nullable_string()
                                },
                                "required":["ordinal","kind","plainText","bounds","latex","tableCells","visualDescription"],
                                "additionalProperties":false
                            }
                        }
                    },
                    "required":["pageNumber","blocks"],
                    "additionalProperties":false
                }
            }
        },
        "required":["schemaVersion","pages"],
        "additionalProperties":false
    })
}

fn invalid_input() -> AppError {
    AppError::new(AppErrorCode::InvalidInput)
}

fn cancelled() -> AppError {
    AiError::cancelled().into_app_error()
}

#[derive(Default)]
struct OpenAiEventMapper {
    saw_visible_text: bool,
}

impl SseEventMapper for OpenAiEventMapper {
    fn map_event(&mut self, event: &SseEvent) -> Result<Vec<UnifiedStreamEvent>, AiError> {
        match event.event_type() {
            "response.output_text.delta" => {
                let payload: OutputTextDelta = parse_known_json(event)?;
                if payload.kind != event.event_type() {
                    return Err(AiError::malformed_event());
                }
                if payload.delta.is_empty() {
                    return Ok(Vec::new());
                }
                self.saw_visible_text = true;
                Ok(vec![UnifiedStreamEvent::TextDelta {
                    text: payload.delta,
                }])
            }
            "response.completed" => {
                let payload: CompletedEvent = parse_known_json(event)?;
                if payload.kind != event.event_type()
                    || payload.response.status != "completed"
                    || !self.saw_visible_text
                {
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
