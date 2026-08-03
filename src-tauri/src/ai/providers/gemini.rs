use std::{collections::BTreeMap, fmt};

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
    structured::{
        decode_provider_page_analysis, validate_provider_page_batch, validate_structured_request,
    },
    transport::{CredentialHeader, ProviderHttpRequest, ProviderTransport},
};

const MODEL_ENDPOINT_PREFIX: &str = "v1beta/models/";
const VERIFIED_VISUAL_MODEL: &str = "gemini-3.6-flash";
const STRUCTURED_MAX_OUTPUT_TOKENS: u32 = 4_096;
const STRUCTURED_INSTRUCTION: &str = "Analyze the supplied textbook page images in their input order and return only the requested JSON object.";

pub struct GeminiProvider {
    transport: ProviderTransport,
    registry: ProviderCapabilityRegistry,
}

impl GeminiProvider {
    pub fn new() -> Result<Self, AiError> {
        Self::build(ProviderTransport::new(ProviderKind::Gemini)?)
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
            ProviderKind::Gemini,
            origin,
        )?)
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
                parts: vec![GeminiPart::Text {
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
            .decode(GeminiEventMapper::for_text()))
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

        let model = request.text.model.clone();
        let body = vision_request(request)?;
        let endpoint = format!(
            "{MODEL_ENDPOINT_PREFIX}{}:streamGenerateContent?alt=sse",
            encode_path_segment(&model).map_err(AiError::into_app_error)?
        );
        let http_request =
            ProviderHttpRequest::json(Method::POST, endpoint, CredentialHeader::XGoogApiKey, &body)
                .map_err(AiError::into_app_error)?;
        drop(body);

        let response = self
            .transport
            .send_stream(http_request, credential, cancel)
            .await
            .map_err(AiError::into_app_error)?;
        Ok(response.decode(GeminiEventMapper::for_vision()))
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

        let model = request.model.clone();
        let schema_version = request.schema_version.clone();
        let max_output_bytes = request.max_output_bytes;
        let expected_page_count = request.pages.len();
        let body = structured_request(request);
        let endpoint = format!(
            "{MODEL_ENDPOINT_PREFIX}{}:generateContent",
            encode_path_segment(&model).map_err(AiError::into_app_error)?
        );
        let http_request =
            ProviderHttpRequest::json(Method::POST, endpoint, CredentialHeader::XGoogApiKey, &body)
                .map_err(AiError::into_app_error)?;
        drop(body);
        let response = self
            .transport
            .send_bounded(http_request, credential, cancel.clone())
            .await
            .map_err(AiError::into_app_error)?;
        let response: GeminiStructuredResponse =
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
        Ok(analysis)
    }
}

impl GeminiProvider {
    fn verified_limits(&self, model: &str, operation: AiOperation) -> AppResult<ImageLimits> {
        if model != VERIFIED_VISUAL_MODEL
            || self
                .registry
                .operation_support(&ProviderKind::Gemini, model, operation)
                != CapabilitySupport::Supported
        {
            return Err(AppError::unsupported_provider_capability());
        }
        self.registry
            .capabilities()
            .iter()
            .find(|provider| provider.kind == ProviderKind::Gemini)
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

fn vision_request(request: UnifiedVisionRequest) -> AppResult<GeminiRequest> {
    let UnifiedVisionRequest { text, images } = request;
    let mut contents = normalize_history(text.messages).map_err(AiError::into_app_error)?;
    let last = contents.last_mut().ok_or_else(invalid_input)?;
    if last.role != Some(GeminiRole::User) {
        return Err(invalid_input());
    }
    let text_parts = std::mem::take(&mut last.parts);
    let mut parts = images
        .into_iter()
        .map(|image| GeminiPart::InlineData {
            inline_data: InlineData {
                mime_type: image_mime(image.meta.mime_type),
                data: SensitiveString::base64(image.bytes()),
            },
        })
        .collect::<Vec<_>>();
    parts.extend(text_parts);
    last.parts = parts;

    Ok(GeminiRequest {
        system_instruction: (!text.system.is_empty()).then(|| GeminiContent {
            role: None,
            parts: vec![GeminiPart::Text { text: text.system }],
        }),
        contents,
        generation_config: GenerationConfig {
            max_output_tokens: text.max_output_tokens,
        },
    })
}

fn structured_request(request: StructuredPageRequest) -> GeminiStructuredRequest {
    let schema_version = request.schema_version.clone();
    let mut parts = request
        .pages
        .into_iter()
        .map(|page| GeminiPart::InlineData {
            inline_data: InlineData {
                mime_type: image_mime(page.meta.mime_type),
                data: SensitiveString::base64(page.bytes()),
            },
        })
        .collect::<Vec<_>>();
    parts.push(GeminiPart::Text {
        text: format!("{STRUCTURED_INSTRUCTION} Return schema version {schema_version}."),
    });
    GeminiStructuredRequest {
        contents: vec![GeminiContent {
            role: Some(GeminiRole::User),
            parts,
        }],
        generation_config: GeminiStructuredConfig {
            max_output_tokens: STRUCTURED_MAX_OUTPUT_TOKENS,
            response_format: GeminiResponseFormat {
                text: GeminiTextResponseFormat {
                    mime_type: "application/json",
                    schema: page_analysis_schema(&schema_version),
                },
            },
        },
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
            previous.parts.push(GeminiPart::Text {
                text: message.content,
            });
        } else {
            contents.push(GeminiContent {
                role: Some(role),
                parts: vec![GeminiPart::Text {
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
    parts: Vec<GeminiPart>,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
enum GeminiRole {
    User,
    Model,
}

#[derive(Serialize)]
#[serde(untagged)]
enum GeminiPart {
    Text {
        text: String,
    },
    InlineData {
        #[serde(rename = "inlineData")]
        inline_data: InlineData,
    },
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct InlineData {
    mime_type: &'static str,
    data: SensitiveString,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GenerationConfig {
    max_output_tokens: u32,
}

struct GeminiEventMapper {
    saw_visible_text: bool,
    allow_max_tokens: bool,
}

impl GeminiEventMapper {
    const fn for_text() -> Self {
        Self {
            saw_visible_text: false,
            allow_max_tokens: true,
        }
    }

    const fn for_vision() -> Self {
        Self {
            saw_visible_text: false,
            allow_max_tokens: false,
        }
    }
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
            && reason != "STOP"
            && !(self.allow_max_tokens && reason == "MAX_TOKENS")
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

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GeminiStructuredRequest {
    contents: Vec<GeminiContent>,
    generation_config: GeminiStructuredConfig,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GeminiStructuredConfig {
    max_output_tokens: u32,
    response_format: GeminiResponseFormat,
}

#[derive(Serialize)]
struct GeminiResponseFormat {
    text: GeminiTextResponseFormat,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GeminiTextResponseFormat {
    mime_type: &'static str,
    schema: Value,
}

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

const fn image_mime(mime_type: ImageMime) -> &'static str {
    match mime_type {
        ImageMime::Png => "image/png",
        ImageMime::Jpeg => "image/jpeg",
        ImageMime::Webp => "image/webp",
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
#[serde(rename_all = "camelCase")]
struct GeminiStructuredResponse {
    candidates: Option<Vec<Value>>,
    prompt_feedback: Option<PromptFeedback>,
    #[serde(default, rename = "usageMetadata")]
    _usage_metadata: Option<IgnoredAny>,
}

impl GeminiStructuredResponse {
    fn visible_json(self) -> AppResult<String> {
        if self
            .prompt_feedback
            .as_ref()
            .is_some_and(|feedback| feedback.block_reason.is_some())
        {
            return Err(AiError::refused().into_app_error());
        }
        let mut candidates = self
            .candidates
            .filter(|candidates| candidates.len() == 1)
            .ok_or_else(|| AiError::provider_unavailable().into_app_error())?;
        let candidate: Candidate = serde_json::from_value(candidates.remove(0))
            .map_err(|_| AiError::provider_unavailable().into_app_error())?;
        if candidate.index.is_some_and(|index| index != 0) {
            return Err(AiError::provider_unavailable().into_app_error());
        }
        match candidate.finish_reason.as_deref() {
            Some("STOP") => {}
            Some(
                "SAFETY" | "RECITATION" | "LANGUAGE" | "BLOCKLIST" | "PROHIBITED_CONTENT" | "SPII"
                | "IMAGE_SAFETY",
            ) => return Err(AiError::refused().into_app_error()),
            _ => return Err(AiError::provider_unavailable().into_app_error()),
        }
        let parts = candidate
            .content
            .and_then(|content| content.parts)
            .ok_or_else(|| AiError::provider_unavailable().into_app_error())?;
        let visible = parts
            .into_iter()
            .filter(|part| !part.thought.unwrap_or(false))
            .filter_map(|part| part.text)
            .collect::<String>();
        if visible.is_empty() {
            Err(AiError::provider_unavailable().into_app_error())
        } else {
            Ok(visible)
        }
    }
}

fn page_analysis_schema(schema_version: &str) -> Value {
    let nullable_string = || json!({"anyOf":[{"type":"string"},{"type":"null"}]});
    json!({
        "type":"object",
        "properties":{
            "schemaVersion":{"type":"string","enum":[schema_version]},
            "pages":{"type":"array","items":{
                "type":"object",
                "properties":{
                    "pageNumber":{"type":"integer","minimum":0},
                    "blocks":{"type":"array","items":{
                        "type":"object",
                        "properties":{
                            "ordinal":{"type":"integer","minimum":0},
                            "kind":{"type":"string","enum":["title","paragraph","list","table","caption","formula","figure","transcript"]},
                            "plainText":{"type":"string"},
                            "bounds":{"anyOf":[{"type":"object","properties":{
                                "x":{"type":"number"},"y":{"type":"number"},
                                "width":{"type":"number"},"height":{"type":"number"}
                            },"required":["x","y","width","height"],"additionalProperties":false},{"type":"null"}]},
                            "latex":nullable_string(),
                            "tableCells":{"anyOf":[{"type":"array","items":{
                                "type":"object","properties":{
                                    "row":{"type":"integer","minimum":0},"column":{"type":"integer","minimum":0},
                                    "rowSpan":{"type":"integer","minimum":1},"columnSpan":{"type":"integer","minimum":1},
                                    "text":{"type":"string"}
                                },"required":["row","column","rowSpan","columnSpan","text"],"additionalProperties":false
                            }},{"type":"null"}]},
                            "visualDescription":nullable_string()
                        },
                        "required":["ordinal","kind","plainText","bounds","latex","tableCells","visualDescription"],
                        "additionalProperties":false
                    }}
                },
                "required":["pageNumber","blocks"],
                "additionalProperties":false
            }}
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
