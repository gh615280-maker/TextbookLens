#[allow(clippy::duplicate_mod)]
#[path = "../../tests/common/provider_server.rs"]
mod provider_server;

use std::{
    io::{self, Write},
    sync::{Arc, Mutex},
    time::Duration,
};

use futures_util::StreamExt;
use secrecy::SecretString;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use tracing_subscriber::fmt::MakeWriter;
use uuid::Uuid;
use wiremock::{
    Mock, ResponseTemplate,
    matchers::{method, path},
};

use super::{
    error::{AiError, AiErrorKind},
    multimodal::stage_vision_asset,
    provider::{
        AiProvider, ProviderStream, UnifiedChatRequest, UnifiedMessage, UnifiedRole,
        UnifiedStreamEvent,
    },
    providers::{
        anthropic::AnthropicProvider, deepseek::DeepSeekProvider, gemini::GeminiProvider,
        kimi::KimiProvider, openai::OpenAiProvider,
    },
    registry::ProviderCapabilityRegistry,
    structured::PAGE_ANALYSIS_SCHEMA_VERSION,
};
use crate::{
    domain::{
        AiOperation, CapabilitySupport, ImageLimits, ImageMime, ProviderKind,
        StructuredPageRequest, UnifiedVisionRequest, VisionAsset,
    },
    errors::{AppError, AppErrorCode, AppErrorDto},
};
use provider_server::{GatedSseServer, ProviderServer};

const WAIT_LIMIT: Duration = Duration::from_secs(5);
const TEST_CREDENTIAL: &str = "operation-contract-synthetic-credential";
const UNKNOWN_MODEL: &str = "operation-contract-unregistered-model";
const RAW_IMAGE_SENTINEL: &str = "operation-contract-raw-image-sentinel";
const VENDOR_SENTINEL: &str = "operation-contract-vendor-body-sentinel";
const HIDDEN_SENTINEL: &str = "operation-contract-hidden-content-sentinel";
const SCHEMA_SENTINEL: &str = PAGE_ANALYSIS_SCHEMA_VERSION;

const TINY_PNG: &[u8] = include_bytes!("../../../fixtures/source/vision/tiny-blue.png");
const TINY_JPEG: &[u8] = include_bytes!("../../../fixtures/source/vision/tiny-orange.jpg");

const OPENAI_TEXT_OK: &str = include_str!("../../../fixtures/providers/openai/stream-ok.sse");
const GEMINI_TEXT_OK: &str = include_str!("../../../fixtures/providers/gemini/stream-ok.sse");
const ANTHROPIC_TEXT_OK: &str = include_str!("../../../fixtures/providers/anthropic/stream-ok.sse");
const DEEPSEEK_TEXT_OK: &str = include_str!("../../../fixtures/providers/deepseek/stream-ok.sse");
const KIMI_TEXT_OK: &str = include_str!("../../../fixtures/providers/kimi/stream-ok.sse");

const OPENAI_VISION_OK: &str =
    include_str!("../../../fixtures/providers/openai/vision-stream-ok.sse");
const GEMINI_VISION_OK: &str =
    include_str!("../../../fixtures/providers/gemini/vision-stream-ok.sse");
const ANTHROPIC_VISION_OK: &str =
    include_str!("../../../fixtures/providers/anthropic/vision-stream-ok.sse");
const KIMI_VISION_OK: &str = include_str!("../../../fixtures/providers/kimi/vision-stream-ok.sse");

const OPENAI_STRUCTURED_OK: &str =
    include_str!("../../../fixtures/providers/openai/structured-ok.json");
const GEMINI_STRUCTURED_OK: &str =
    include_str!("../../../fixtures/providers/gemini/structured-ok.json");
const ANTHROPIC_STRUCTURED_OK: &str =
    include_str!("../../../fixtures/providers/anthropic/structured-ok.json");
const KIMI_STRUCTURED_OK: &str =
    include_str!("../../../fixtures/providers/kimi/structured-ok.json");

#[derive(Clone, Copy, Debug)]
enum OperationProvider {
    OpenAi,
    Gemini,
    Anthropic,
    DeepSeek,
    Kimi,
}

const PROVIDERS: [OperationProvider; 5] = [
    OperationProvider::OpenAi,
    OperationProvider::Gemini,
    OperationProvider::Anthropic,
    OperationProvider::DeepSeek,
    OperationProvider::Kimi,
];

const VISUAL_PROVIDERS: [OperationProvider; 4] = [
    OperationProvider::OpenAi,
    OperationProvider::Gemini,
    OperationProvider::Anthropic,
    OperationProvider::Kimi,
];

impl OperationProvider {
    fn kind(self) -> ProviderKind {
        match self {
            Self::OpenAi => ProviderKind::OpenAi,
            Self::Gemini => ProviderKind::Gemini,
            Self::Anthropic => ProviderKind::Anthropic,
            Self::DeepSeek => ProviderKind::DeepSeek,
            Self::Kimi => ProviderKind::Kimi,
        }
    }

    fn model(self) -> &'static str {
        match self {
            Self::OpenAi => "gpt-5.6",
            Self::Gemini => "gemini-3.6-flash",
            Self::Anthropic => "claude-sonnet-5",
            Self::DeepSeek => "deepseek-v4-flash",
            Self::Kimi => "kimi-k3",
        }
    }

    fn provider(self, loopback_origin: &str) -> Box<dyn AiProvider> {
        match self {
            Self::OpenAi => Box::new(OpenAiProvider::new_for_test(loopback_origin).unwrap()),
            Self::Gemini => Box::new(GeminiProvider::new_for_test(loopback_origin).unwrap()),
            Self::Anthropic => Box::new(AnthropicProvider::new_for_test(loopback_origin).unwrap()),
            Self::DeepSeek => Box::new(DeepSeekProvider::new_for_test(loopback_origin).unwrap()),
            Self::Kimi => {
                Box::new(KimiProvider::new_for_test(&format!("{loopback_origin}/v1")).unwrap())
            }
        }
    }

    fn text_path(self) -> &'static str {
        match self {
            Self::OpenAi => "/v1/responses",
            Self::Gemini => "/v1beta/models/gemini-3.6-flash:streamGenerateContent",
            Self::Anthropic => "/v1/messages",
            Self::DeepSeek => "/chat/completions",
            Self::Kimi => "/v1/chat/completions",
        }
    }

    fn vision_path(self) -> &'static str {
        match self {
            Self::OpenAi => "/v1/responses",
            Self::Gemini => "/v1beta/models/gemini-3.6-flash:streamGenerateContent",
            Self::Anthropic => "/v1/messages",
            Self::DeepSeek => "/chat/completions",
            Self::Kimi => "/v1/chat/completions",
        }
    }

    fn structured_path(self) -> &'static str {
        match self {
            Self::OpenAi => "/v1/responses",
            Self::Gemini => "/v1beta/models/gemini-3.6-flash:generateContent",
            Self::Anthropic => "/v1/messages",
            Self::DeepSeek => "/chat/completions",
            Self::Kimi => "/v1/chat/completions",
        }
    }

    fn text_fixture(self) -> &'static str {
        match self {
            Self::OpenAi => OPENAI_TEXT_OK,
            Self::Gemini => GEMINI_TEXT_OK,
            Self::Anthropic => ANTHROPIC_TEXT_OK,
            Self::DeepSeek => DEEPSEEK_TEXT_OK,
            Self::Kimi => KIMI_TEXT_OK,
        }
    }

    fn vision_fixture(self) -> &'static str {
        match self {
            Self::OpenAi => OPENAI_VISION_OK,
            Self::Gemini => GEMINI_VISION_OK,
            Self::Anthropic => ANTHROPIC_VISION_OK,
            Self::Kimi => KIMI_VISION_OK,
            Self::DeepSeek => panic!("DeepSeek has no visual fixture"),
        }
    }

    fn structured_fixture(self) -> &'static str {
        match self {
            Self::OpenAi => OPENAI_STRUCTURED_OK,
            Self::Gemini => GEMINI_STRUCTURED_OK,
            Self::Anthropic => ANTHROPIC_STRUCTURED_OK,
            Self::Kimi => KIMI_STRUCTURED_OK,
            Self::DeepSeek => panic!("DeepSeek has no structured page fixture"),
        }
    }

    fn first_vision_frames(self) -> Vec<u8> {
        match self {
            Self::OpenAi => b"event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"first\"}\n\n".to_vec(),
            Self::Gemini => b"data: {\"candidates\":[{\"index\":0,\"content\":{\"parts\":[{\"text\":\"first\"}]}}]}\n\n".to_vec(),
            Self::Anthropic => concat!(
                "event: message_start\n",
                "data: {\"message\":{\"type\":\"message\",\"usage\":{\"input_tokens\":1}}}\n\n",
                "event: content_block_start\n",
                "data: {\"content_block\":{\"type\":\"text\"}}\n\n",
                "event: content_block_delta\n",
                "data: {\"delta\":{\"type\":\"text_delta\",\"text\":\"first\"}}\n\n"
            )
            .as_bytes()
            .to_vec(),
            Self::Kimi => b"data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"first\"},\"finish_reason\":null}]}\n\n".to_vec(),
            Self::DeepSeek => panic!("DeepSeek has no visual stream"),
        }
    }

    fn terminal_vision_frames(self) -> Vec<u8> {
        match self {
            Self::OpenAi => b"event: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\",\"usage\":{\"input_tokens\":1,\"output_tokens\":1}}}\n\n".to_vec(),
            Self::Gemini => b"data: {\"candidates\":[{\"index\":0,\"content\":{\"parts\":[{\"text\":\"terminal\"}]},\"finishReason\":\"STOP\"}],\"usageMetadata\":{\"promptTokenCount\":1,\"candidatesTokenCount\":1}}\n\n".to_vec(),
            Self::Anthropic => concat!(
                "event: content_block_stop\n",
                "data: {}\n\n",
                "event: message_delta\n",
                "data: {\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":1}}\n\n",
                "event: message_stop\n",
                "data: {}\n\n"
            )
            .as_bytes()
            .to_vec(),
            Self::Kimi => concat!(
                "data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
                "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":1,\"completion_tokens\":1}}\n\n",
                "data: [DONE]\n\n"
            )
            .as_bytes()
            .to_vec(),
            Self::DeepSeek => panic!("DeepSeek has no visual stream"),
        }
    }
}

#[tokio::test]
async fn operation_contract_text_does_not_regress_for_any_registered_profile() {
    let registry = ProviderCapabilityRegistry::load_embedded().unwrap();
    for case in PROVIDERS {
        assert_eq!(
            registry.operation_support(&case.kind(), case.model(), AiOperation::TextLearning),
            CapabilitySupport::Supported,
            "{case:?}"
        );

        let server = ProviderServer::start().await;
        Mock::given(method("POST"))
            .and(path(case.text_path()))
            .respond_with(sse_response(case.text_fixture()))
            .expect(1)
            .mount(server.mock_server())
            .await;
        let events = case
            .provider(&server.uri())
            .stream_chat(
                &SecretString::from(TEST_CREDENTIAL),
                text_request(case.model()),
                CancellationToken::new(),
            )
            .await
            .unwrap_or_else(|error| panic!("{case:?} text start failed: {error:?}"))
            .collect::<Vec<_>>()
            .await;
        assert!(events.iter().all(Result::is_ok), "{case:?}: {events:?}");
        assert!(
            events.iter().any(|event| matches!(
                event,
                Ok(UnifiedStreamEvent::TextDelta { text }) if !text.is_empty()
            )),
            "{case:?}: {events:?}"
        );
        assert_eq!(completion_count(&events), 1, "{case:?}: {events:?}");
    }
}

#[tokio::test]
async fn operation_contract_supported_operations_match_exact_capability_truth() {
    let registry = ProviderCapabilityRegistry::load_embedded().unwrap();
    for case in PROVIDERS {
        let expected = if matches!(case, OperationProvider::DeepSeek) {
            CapabilitySupport::Unsupported
        } else {
            CapabilitySupport::Supported
        };
        assert_eq!(
            registry.operation_support(&case.kind(), case.model(), AiOperation::VisionLearning),
            expected,
            "{case:?} vision truth"
        );
        assert_eq!(
            registry.operation_support(
                &case.kind(),
                case.model(),
                AiOperation::StructuredPageAnalysis,
            ),
            expected,
            "{case:?} structured truth"
        );
    }

    for case in VISUAL_PROVIDERS {
        let vision_server = ProviderServer::start().await;
        Mock::given(method("POST"))
            .and(path(case.vision_path()))
            .respond_with(sse_response(case.vision_fixture()))
            .expect(1)
            .mount(vision_server.mock_server())
            .await;
        let events = case
            .provider(&vision_server.uri())
            .stream_vision(
                &SecretString::from(TEST_CREDENTIAL),
                vision_request(case.model()),
                CancellationToken::new(),
            )
            .await
            .unwrap_or_else(|error| panic!("{case:?} vision start failed: {error:?}"))
            .collect::<Vec<_>>()
            .await;
        assert!(events.iter().all(Result::is_ok), "{case:?}: {events:?}");
        assert_eq!(completion_count(&events), 1, "{case:?}: {events:?}");
        let visible_events = events
            .iter()
            .map(|event| event.as_ref().unwrap())
            .collect::<Vec<_>>();
        let event_json = serde_json::to_string(&visible_events).unwrap();
        for hidden in [
            "hidden-vision-thinking",
            "hidden-vision-signature",
            "hidden-thought",
            "hidden-signature",
            "hidden-vision-reasoning",
        ] {
            assert!(!event_json.contains(hidden), "{case:?} leaked {hidden}");
        }
        let requests = vision_server
            .mock_server()
            .received_requests()
            .await
            .unwrap();
        assert_eq!(requests.len(), 1, "{case:?} must use one inline request");
        assert_ne!(requests[0].method.as_str(), "DELETE", "{case:?}");
        assert!(!requests[0].url.path().contains("/files"), "{case:?}");

        let structured_server = ProviderServer::start().await;
        Mock::given(method("POST"))
            .and(path(case.structured_path()))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_raw(case.structured_fixture(), "application/json"),
            )
            .expect(1)
            .mount(structured_server.mock_server())
            .await;
        let analysis = case
            .provider(&structured_server.uri())
            .analyze_pages(
                &SecretString::from(TEST_CREDENTIAL),
                structured_request(case.model(), 1, 4_096),
                CancellationToken::new(),
            )
            .await
            .unwrap_or_else(|error| panic!("{case:?} structured failed: {error:?}"));
        assert_eq!(
            analysis.analysis.schema_version,
            PAGE_ANALYSIS_SCHEMA_VERSION
        );
        assert_eq!(analysis.analysis.pages.len(), 1);
        assert!(analysis.cleanup.is_none());
        let analysis_json = serde_json::to_string(&analysis.analysis).unwrap();
        for hidden in [
            "hidden-structured-thinking",
            "hidden-structured-signature",
            "hidden-structured-thought",
            "hidden-structured-reasoning",
        ] {
            assert!(!analysis_json.contains(hidden), "{case:?} leaked {hidden}");
        }
        let requests = structured_server
            .mock_server()
            .received_requests()
            .await
            .unwrap();
        assert_eq!(requests.len(), 1, "{case:?} must use one inline request");
        assert_ne!(requests[0].method.as_str(), "DELETE", "{case:?}");
        assert!(!requests[0].url.path().contains("/files"), "{case:?}");
    }
}

#[tokio::test]
async fn operation_contract_unknown_and_unsupported_deny_before_credential_or_network() {
    for case in PROVIDERS {
        let server = ProviderServer::start().await;
        let provider = case.provider(&server.uri());
        let empty_credential = SecretString::from("");
        let vision_error = match provider
            .stream_vision(
                &empty_credential,
                vision_request(UNKNOWN_MODEL),
                CancellationToken::new(),
            )
            .await
        {
            Err(error) => error,
            Ok(_) => panic!("{case:?} accepted unknown-model vision"),
        };
        let structured_error = provider
            .analyze_pages(
                &empty_credential,
                structured_request(UNKNOWN_MODEL, 1, 4_096),
                CancellationToken::new(),
            )
            .await
            .unwrap_err();
        assert_unsupported(&vision_error, case);
        assert_unsupported(&structured_error, case);
        assert!(
            server
                .mock_server()
                .received_requests()
                .await
                .unwrap()
                .is_empty(),
            "{case:?} touched the network for unknown capability"
        );
    }

    let server = ProviderServer::start().await;
    let provider = OperationProvider::DeepSeek.provider(&server.uri());
    let empty_credential = SecretString::from("");
    let vision_error = match provider
        .stream_vision(
            &empty_credential,
            vision_request(OperationProvider::DeepSeek.model()),
            CancellationToken::new(),
        )
        .await
    {
        Err(error) => error,
        Ok(_) => panic!("DeepSeek accepted its unsupported visual operation"),
    };
    let structured_error = provider
        .analyze_pages(
            &empty_credential,
            structured_request(OperationProvider::DeepSeek.model(), 1, 4_096),
            CancellationToken::new(),
        )
        .await
        .unwrap_err();
    assert_unsupported(&vision_error, OperationProvider::DeepSeek);
    assert_unsupported(&structured_error, OperationProvider::DeepSeek);
    assert!(
        server
            .mock_server()
            .received_requests()
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn operation_contract_corrupt_wrong_version_oversize_missing_duplicate_hidden_refusal_and_eof_never_return_pages()
 {
    for case in VISUAL_PROVIDERS {
        for vector in structured_failure_vectors(case) {
            let server = ProviderServer::start().await;
            Mock::given(method("POST"))
                .and(path(case.structured_path()))
                .respond_with(
                    ResponseTemplate::new(200).set_body_raw(vector.body, "application/json"),
                )
                .expect(1)
                .mount(server.mock_server())
                .await;
            let result = case
                .provider(&server.uri())
                .analyze_pages(
                    &SecretString::from(TEST_CREDENTIAL),
                    structured_request(case.model(), vector.page_count, vector.max_output_bytes),
                    CancellationToken::new(),
                )
                .await;
            assert!(
                result.is_err(),
                "{case:?} {} returned page analysis",
                vector.name
            );
            let requests = server.mock_server().received_requests().await.unwrap();
            assert_eq!(requests.len(), 1, "{case:?} {}", vector.name);
            assert_ne!(requests[0].method.as_str(), "DELETE", "{case:?}");
            assert!(!requests[0].url.path().contains("/files"), "{case:?}");
        }
    }
}

#[tokio::test]
async fn operation_contract_cancel_before_encode_never_sends_or_cleans_up() {
    for case in VISUAL_PROVIDERS {
        let server = ProviderServer::start().await;
        let provider = case.provider(&server.uri());

        let vision_cancel = CancellationToken::new();
        vision_cancel.cancel();
        let vision_error = match provider
            .stream_vision(
                &SecretString::from(TEST_CREDENTIAL),
                vision_request(case.model()),
                vision_cancel,
            )
            .await
        {
            Err(error) => error,
            Ok(_) => panic!("{case:?} returned a stream after pre-encode cancellation"),
        };
        assert_eq!(vision_error.code, AppErrorCode::ImportCancelled, "{case:?}");

        let structured_cancel = CancellationToken::new();
        structured_cancel.cancel();
        let structured_error = provider
            .analyze_pages(
                &SecretString::from(TEST_CREDENTIAL),
                structured_request(case.model(), 1, 4_096),
                structured_cancel,
            )
            .await
            .unwrap_err();
        assert_eq!(
            structured_error.code,
            AppErrorCode::ImportCancelled,
            "{case:?}"
        );
        assert!(
            server
                .mock_server()
                .received_requests()
                .await
                .unwrap()
                .is_empty(),
            "{case:?} encoded/sent or attempted cleanup after pre-encode cancellation"
        );
    }
}

#[tokio::test]
async fn operation_contract_cancel_between_frames_emits_nothing_late() {
    for case in VISUAL_PROVIDERS {
        let server =
            GatedSseServer::start(case.first_vision_frames(), case.terminal_vision_frames()).await;
        let cancel = CancellationToken::new();
        let mut stream = start_gated_vision(case, &server, cancel.clone()).await;
        tokio::time::timeout(WAIT_LIMIT, server.first_written.notified())
            .await
            .unwrap_or_else(|_| panic!("{case:?} first frame was not written"));
        expect_first_visible(case, &mut stream).await;
        cancel.cancel();
        expect_cancel_then_end(case, &mut stream).await;
        server.allow_terminal.notify_one();
        assert_eq!(server.request_count(), 1, "{case:?}");
    }
}

#[tokio::test]
async fn operation_contract_cancel_after_terminal_parse_before_return_drops_completion() {
    for case in VISUAL_PROVIDERS {
        let server =
            GatedSseServer::start(case.first_vision_frames(), case.terminal_vision_frames()).await;
        let cancel = CancellationToken::new();
        let mut stream = start_gated_vision(case, &server, cancel.clone()).await;
        expect_first_visible(case, &mut stream).await;
        server.allow_terminal.notify_one();
        tokio::time::timeout(WAIT_LIMIT, server.terminal_written.notified())
            .await
            .unwrap_or_else(|_| panic!("{case:?} terminal frames were not written"));

        let boundary = tokio::time::timeout(WAIT_LIMIT, stream.next())
            .await
            .unwrap_or_else(|_| panic!("{case:?} terminal parse timed out"))
            .unwrap_or_else(|| panic!("{case:?} ended before a cancellable terminal boundary"))
            .unwrap_or_else(|error| panic!("{case:?} terminal parse failed: {error:?}"));
        assert!(
            !matches!(boundary, UnifiedStreamEvent::Completed),
            "{case:?} exposed completion without a cancellable return boundary"
        );
        cancel.cancel();
        expect_cancel_then_end(case, &mut stream).await;
        assert_eq!(server.request_count(), 1, "{case:?}");
    }
}

#[tokio::test]
async fn operation_contract_raw_image_schema_vendor_credential_and_hidden_content_never_leak() {
    for case in VISUAL_PROVIDERS {
        let server = ProviderServer::start().await;
        Mock::given(method("POST"))
            .and(path(case.structured_path()))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_raw(sensitive_refusal_response(case), "application/json"),
            )
            .expect(1)
            .mount(server.mock_server())
            .await;

        let error = case
            .provider(&server.uri())
            .analyze_pages(
                &SecretString::from(TEST_CREDENTIAL),
                sensitive_structured_request(case.model()),
                CancellationToken::new(),
            )
            .await
            .unwrap_err();
        for surface in safe_error_surfaces(error) {
            for sentinel in [
                RAW_IMAGE_SENTINEL,
                SCHEMA_SENTINEL,
                VENDOR_SENTINEL,
                TEST_CREDENTIAL,
                HIDDEN_SENTINEL,
            ] {
                assert!(
                    !surface.contains(sentinel),
                    "{case:?} leaked {sentinel} through a safe surface"
                );
            }
        }
        let requests = server.mock_server().received_requests().await.unwrap();
        assert_eq!(requests.len(), 1, "{case:?}");
        assert_ne!(requests[0].method.as_str(), "DELETE", "{case:?}");
        assert!(!requests[0].url.path().contains("/files"), "{case:?}");
    }
}

fn text_request(model: &str) -> UnifiedChatRequest {
    UnifiedChatRequest {
        model: model.to_owned(),
        system: "operation-contract-system".to_owned(),
        messages: vec![UnifiedMessage {
            role: UnifiedRole::User,
            content: "operation-contract-question".to_owned(),
        }],
        max_output_tokens: 321,
        expected_language: Some("en".to_owned()),
    }
}

fn vision_request(model: &str) -> UnifiedVisionRequest {
    UnifiedVisionRequest {
        text: text_request(model),
        images: vec![fixture_asset(
            Uuid::new_v4(),
            Uuid::new_v4(),
            ImageMime::Png,
            TINY_PNG.to_vec(),
        )],
    }
}

fn structured_request(
    model: &str,
    page_count: usize,
    max_output_bytes: u32,
) -> StructuredPageRequest {
    let book_id = Uuid::new_v4();
    let pages = (0..page_count)
        .map(|index| {
            if index % 2 == 0 {
                fixture_asset(
                    book_id,
                    Uuid::new_v4(),
                    ImageMime::Png,
                    with_unique_suffix(TINY_PNG, index),
                )
            } else {
                fixture_asset(
                    book_id,
                    Uuid::new_v4(),
                    ImageMime::Jpeg,
                    with_unique_suffix(TINY_JPEG, index),
                )
            }
        })
        .collect();
    StructuredPageRequest {
        model: model.to_owned(),
        pages,
        schema_version: PAGE_ANALYSIS_SCHEMA_VERSION.to_owned(),
        max_output_bytes,
    }
}

fn sensitive_structured_request(model: &str) -> StructuredPageRequest {
    let mut bytes = TINY_PNG.to_vec();
    bytes.extend_from_slice(RAW_IMAGE_SENTINEL.as_bytes());
    StructuredPageRequest {
        model: model.to_owned(),
        pages: vec![fixture_asset(
            Uuid::new_v4(),
            Uuid::new_v4(),
            ImageMime::Png,
            bytes,
        )],
        schema_version: PAGE_ANALYSIS_SCHEMA_VERSION.to_owned(),
        max_output_bytes: 4_096,
    }
}

fn fixture_asset(
    book_id: Uuid,
    capture_id: Uuid,
    mime_type: ImageMime,
    bytes: Vec<u8>,
) -> VisionAsset {
    stage_vision_asset(book_id, capture_id, mime_type, 2, 2, bytes, image_limits()).unwrap()
}

fn with_unique_suffix(bytes: &[u8], index: usize) -> Vec<u8> {
    let mut output = bytes.to_vec();
    output.extend_from_slice(format!("-operation-page-{index}").as_bytes());
    output
}

const fn image_limits() -> ImageLimits {
    ImageLimits {
        max_images: 4,
        max_encoded_bytes_each: 4 * 1024 * 1024,
        max_total_encoded_bytes: 12 * 1024 * 1024,
        max_dimension_px: 4_096,
        max_decoded_pixels_each: 8_847_360,
    }
}

fn sse_response(body: &'static str) -> ResponseTemplate {
    ResponseTemplate::new(200)
        .insert_header("content-type", "text/event-stream")
        .set_body_raw(body, "text/event-stream")
}

fn completion_count(events: &[Result<UnifiedStreamEvent, AiError>]) -> usize {
    events
        .iter()
        .filter(|event| matches!(event, Ok(UnifiedStreamEvent::Completed)))
        .count()
}

fn assert_unsupported(error: &AppError, case: OperationProvider) {
    assert_eq!(
        error.stable_code(),
        "UNSUPPORTED_PROVIDER_CAPABILITY",
        "{case:?}: {error:?}"
    );
}

struct StructuredFailureVector {
    name: &'static str,
    body: String,
    page_count: usize,
    max_output_bytes: u32,
}

fn structured_failure_vectors(case: OperationProvider) -> Vec<StructuredFailureVector> {
    let wrong_version = analysis_json("textbooklens.page-analysis.v2", vec![page_json(1)]);
    let oversized = analysis_json(
        PAGE_ANALYSIS_SCHEMA_VERSION,
        vec![page_with_text(1, &"x".repeat(512))],
    );
    let missing = analysis_json(PAGE_ANALYSIS_SCHEMA_VERSION, Vec::new());
    let duplicate = analysis_json(
        PAGE_ANALYSIS_SCHEMA_VERSION,
        vec![page_json(1), page_json(1)],
    );
    vec![
        StructuredFailureVector {
            name: "corrupt JSON",
            body: "{".to_owned(),
            page_count: 1,
            max_output_bytes: 4_096,
        },
        StructuredFailureVector {
            name: "wrong schema version",
            body: structured_success_response(case, &wrong_version),
            page_count: 1,
            max_output_bytes: 4_096,
        },
        StructuredFailureVector {
            name: "oversize visible result",
            body: structured_success_response(case, &oversized),
            page_count: 1,
            max_output_bytes: 256,
        },
        StructuredFailureVector {
            name: "missing pages",
            body: structured_success_response(case, &missing),
            page_count: 1,
            max_output_bytes: 4_096,
        },
        StructuredFailureVector {
            name: "duplicate pages",
            body: structured_success_response(case, &duplicate),
            page_count: 2,
            max_output_bytes: 4_096,
        },
        StructuredFailureVector {
            name: "hidden-only result",
            body: hidden_only_response(case),
            page_count: 1,
            max_output_bytes: 4_096,
        },
        StructuredFailureVector {
            name: "refusal",
            body: refusal_response(case),
            page_count: 1,
            max_output_bytes: 4_096,
        },
        StructuredFailureVector {
            name: "EOF",
            body: String::new(),
            page_count: 1,
            max_output_bytes: 4_096,
        },
    ]
}

fn analysis_json(schema_version: &str, pages: Vec<Value>) -> String {
    json!({"schemaVersion": schema_version, "pages": pages}).to_string()
}

fn page_json(page_number: u32) -> Value {
    json!({"pageNumber": page_number, "blocks": []})
}

fn page_with_text(page_number: u32, text: &str) -> Value {
    json!({
        "pageNumber": page_number,
        "blocks": [{
            "ordinal": 0,
            "kind": "paragraph",
            "plainText": text,
            "bounds": null,
            "latex": null,
            "tableCells": null,
            "visualDescription": null
        }]
    })
}

fn structured_success_response(case: OperationProvider, visible: &str) -> String {
    match case {
        OperationProvider::OpenAi => json!({
            "status": "completed",
            "output": [{"type": "message", "content": [{"type": "output_text", "text": visible}]}]
        })
        .to_string(),
        OperationProvider::Gemini => json!({
            "candidates": [{
                "index": 0,
                "content": {"parts": [{"text": visible}]},
                "finishReason": "STOP"
            }]
        })
        .to_string(),
        OperationProvider::Anthropic => json!({
            "type": "message",
            "role": "assistant",
            "content": [{"type": "text", "text": visible}],
            "stop_reason": "end_turn"
        })
        .to_string(),
        OperationProvider::Kimi => json!({
            "choices": [{
                "index": 0,
                "finish_reason": "stop",
                "message": {"content": visible}
            }]
        })
        .to_string(),
        OperationProvider::DeepSeek => panic!("DeepSeek has no structured response"),
    }
}

fn hidden_only_response(case: OperationProvider) -> String {
    match case {
        OperationProvider::OpenAi => json!({
            "status": "completed",
            "output": [{"type": "reasoning", "summary": HIDDEN_SENTINEL}]
        })
        .to_string(),
        OperationProvider::Gemini => json!({
            "candidates": [{
                "index": 0,
                "content": {"parts": [{"thought": true, "text": HIDDEN_SENTINEL}]},
                "finishReason": "STOP"
            }]
        })
        .to_string(),
        OperationProvider::Anthropic => json!({
            "type": "message",
            "role": "assistant",
            "content": [{"type": "thinking", "thinking": HIDDEN_SENTINEL}],
            "stop_reason": "end_turn"
        })
        .to_string(),
        OperationProvider::Kimi => json!({
            "choices": [{
                "index": 0,
                "finish_reason": "stop",
                "message": {"content": null, "reasoning_content": HIDDEN_SENTINEL}
            }]
        })
        .to_string(),
        OperationProvider::DeepSeek => panic!("DeepSeek has no structured response"),
    }
}

fn refusal_response(case: OperationProvider) -> String {
    match case {
        OperationProvider::OpenAi => json!({
            "status": "completed",
            "output": [{"type": "message", "content": [{"type": "refusal", "refusal": VENDOR_SENTINEL}]}]
        })
        .to_string(),
        OperationProvider::Gemini => json!({
            "promptFeedback": {"blockReason": "SAFETY", "blockReasonMessage": VENDOR_SENTINEL}
        })
        .to_string(),
        OperationProvider::Anthropic => json!({
            "type": "message",
            "role": "assistant",
            "content": [{"type": "text", "text": VENDOR_SENTINEL}],
            "stop_reason": "refusal"
        })
        .to_string(),
        OperationProvider::Kimi => json!({
            "choices": [{
                "index": 0,
                "finish_reason": "content_filter",
                "message": {"content": VENDOR_SENTINEL}
            }]
        })
        .to_string(),
        OperationProvider::DeepSeek => panic!("DeepSeek has no structured response"),
    }
}

fn sensitive_refusal_response(case: OperationProvider) -> String {
    match case {
        OperationProvider::OpenAi => json!({
            "status": "completed",
            "schema": SCHEMA_SENTINEL,
            "vendor": VENDOR_SENTINEL,
            "output": [
                {"type": "reasoning", "summary": HIDDEN_SENTINEL},
                {"type": "message", "content": [{"type": "refusal", "refusal": VENDOR_SENTINEL}]}
            ]
        })
        .to_string(),
        OperationProvider::Gemini => json!({
            "schema": SCHEMA_SENTINEL,
            "vendor": VENDOR_SENTINEL,
            "promptFeedback": {
                "blockReason": "SAFETY",
                "blockReasonMessage": VENDOR_SENTINEL,
                "hidden": HIDDEN_SENTINEL
            }
        })
        .to_string(),
        OperationProvider::Anthropic => json!({
            "type": "message",
            "role": "assistant",
            "schema": SCHEMA_SENTINEL,
            "vendor": VENDOR_SENTINEL,
            "content": [
                {"type": "thinking", "thinking": HIDDEN_SENTINEL},
                {"type": "text", "text": VENDOR_SENTINEL}
            ],
            "stop_reason": "refusal"
        })
        .to_string(),
        OperationProvider::Kimi => json!({
            "schema": SCHEMA_SENTINEL,
            "vendor": VENDOR_SENTINEL,
            "choices": [{
                "index": 0,
                "finish_reason": "content_filter",
                "message": {
                    "content": VENDOR_SENTINEL,
                    "reasoning_content": HIDDEN_SENTINEL
                }
            }]
        })
        .to_string(),
        OperationProvider::DeepSeek => panic!("DeepSeek has no structured response"),
    }
}

async fn start_gated_vision(
    case: OperationProvider,
    server: &GatedSseServer,
    cancel: CancellationToken,
) -> ProviderStream {
    tokio::time::timeout(
        WAIT_LIMIT,
        case.provider(server.uri()).stream_vision(
            &SecretString::from(TEST_CREDENTIAL),
            vision_request(case.model()),
            cancel,
        ),
    )
    .await
    .unwrap_or_else(|_| panic!("{case:?} timed out waiting for stream headers"))
    .unwrap_or_else(|error| panic!("{case:?} failed before stream: {error:?}"))
}

async fn expect_first_visible(case: OperationProvider, stream: &mut ProviderStream) {
    loop {
        let event = tokio::time::timeout(WAIT_LIMIT, stream.next())
            .await
            .unwrap_or_else(|_| panic!("{case:?} timed out waiting for the first delta"))
            .unwrap_or_else(|| panic!("{case:?} ended before the first delta"))
            .unwrap_or_else(|error| panic!("{case:?} failed before the first delta: {error:?}"));
        match event {
            UnifiedStreamEvent::TextDelta { text } => {
                assert_eq!(text, "first", "{case:?}");
                return;
            }
            UnifiedStreamEvent::Usage { .. } => {}
            UnifiedStreamEvent::Completed => panic!("{case:?} completed before visible text"),
        }
    }
}

async fn expect_cancel_then_end(case: OperationProvider, stream: &mut ProviderStream) {
    let error = tokio::time::timeout(WAIT_LIMIT, stream.next())
        .await
        .unwrap_or_else(|_| panic!("{case:?} cancellation timed out"))
        .unwrap_or_else(|| panic!("{case:?} ended without a cancellation error"))
        .unwrap_err();
    assert_eq!(error.kind(), AiErrorKind::Cancelled, "{case:?}");
    assert!(
        tokio::time::timeout(WAIT_LIMIT, stream.next())
            .await
            .unwrap_or_else(|_| panic!("{case:?} did not end after cancellation"))
            .is_none(),
        "{case:?} emitted after cancellation"
    );
}

#[derive(Clone, Default)]
struct LogCapture(Arc<Mutex<Vec<u8>>>);

impl LogCapture {
    fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

struct LogWriter(Arc<Mutex<Vec<u8>>>);

impl Write for LogWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'writer> MakeWriter<'writer> for LogCapture {
    type Writer = LogWriter;

    fn make_writer(&'writer self) -> Self::Writer {
        LogWriter(self.0.clone())
    }
}

fn safe_error_surfaces(error: AppError) -> Vec<String> {
    let debug = format!("{error:?}");
    let display = error.to_string();
    let source = std::error::Error::source(&error)
        .map(ToString::to_string)
        .unwrap_or_default();
    let capture = LogCapture::default();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_writer(capture.clone())
        .finish();
    let dto = tracing::subscriber::with_default(subscriber, || AppErrorDto::from(error));
    vec![
        debug,
        display,
        source,
        serde_json::to_string(&dto).unwrap(),
        capture.text(),
    ]
}
