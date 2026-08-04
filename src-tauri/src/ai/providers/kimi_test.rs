#[allow(clippy::duplicate_mod)]
#[path = "../../../tests/common/provider_server.rs"]
mod provider_server;

use futures_util::StreamExt;
use secrecy::SecretString;
use serde_json::json;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use wiremock::{
    Mock, ResponseTemplate,
    matchers::{body_json, header, method, path},
};

use super::kimi::KimiProvider;
use crate::{
    ai::{
        error::AiErrorKind,
        multimodal::stage_vision_asset,
        provider::{
            AiProvider, UnifiedChatRequest, UnifiedMessage, UnifiedRole, UnifiedStreamEvent,
        },
        structured::PAGE_ANALYSIS_SCHEMA_VERSION,
    },
    domain::{ImageLimits, ImageMime, StructuredPageRequest, UnifiedVisionRequest},
    errors::AppErrorCode,
};
use provider_server::{ChunkedSseServer, ProviderServer};

const KEY: &str = "test-kimi-key-not-secret";
const MODELS: &str = include_str!("../../../../fixtures/providers/kimi/validate-ok.json");
const STREAM: &str = include_str!("../../../../fixtures/providers/kimi/stream-ok.sse");
const ERROR: &str = include_str!("../../../../fixtures/providers/kimi/stream-error.sse");
const VISION_STREAM_OK: &str =
    include_str!("../../../../fixtures/providers/kimi/vision-stream-ok.sse");
const VISION_STREAM_TRUNCATED: &str =
    include_str!("../../../../fixtures/providers/kimi/vision-stream-truncated.sse");
const STRUCTURED_OK: &str = include_str!("../../../../fixtures/providers/kimi/structured-ok.json");
const STRUCTURED_REFUSAL: &str =
    include_str!("../../../../fixtures/providers/kimi/structured-refusal.json");
const STRUCTURED_TRUNCATED: &str =
    include_str!("../../../../fixtures/providers/kimi/structured-truncated.json");
const SYNTHETIC_PNG_BASE64: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAAB";
fn request() -> UnifiedChatRequest {
    UnifiedChatRequest {
        model: "fixture-kimi-model".into(),
        system: "fixture-system".into(),
        messages: vec![
            UnifiedMessage {
                role: UnifiedRole::User,
                content: "fixture question".into(),
            },
            UnifiedMessage {
                role: UnifiedRole::Assistant,
                content: "prior answer".into(),
            },
        ],
        max_output_tokens: 321,
        expected_language: None,
    }
}

fn sse_response(body: &'static str) -> ResponseTemplate {
    ResponseTemplate::new(200)
        .insert_header("content-type", "text/event-stream")
        .set_body_raw(body, "text/event-stream")
}

fn visual_request(model: &str) -> UnifiedVisionRequest {
    UnifiedVisionRequest {
        text: UnifiedChatRequest {
            model: model.to_owned(),
            system: "fixture-visual-system".to_owned(),
            messages: vec![
                UnifiedMessage {
                    role: UnifiedRole::User,
                    content: "fixture-prior-question".to_owned(),
                },
                UnifiedMessage {
                    role: UnifiedRole::Assistant,
                    content: "fixture-prior-answer".to_owned(),
                },
                UnifiedMessage {
                    role: UnifiedRole::User,
                    content: "fixture-visible-image-question".to_owned(),
                },
            ],
            max_output_tokens: 321,
            expected_language: None,
        },
        images: vec![synthetic_image(Uuid::new_v4())],
    }
}

fn structured_request(model: &str) -> StructuredPageRequest {
    StructuredPageRequest {
        model: model.to_owned(),
        pages: vec![synthetic_image(Uuid::new_v4())],
        schema_version: PAGE_ANALYSIS_SCHEMA_VERSION.to_owned(),
        max_output_bytes: 4_096,
    }
}

fn synthetic_image(book_id: Uuid) -> crate::domain::VisionAsset {
    stage_vision_asset(
        book_id,
        Uuid::new_v4(),
        ImageMime::Png,
        1,
        1,
        b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0dIHDR\x00\x00\x00\x01\x00\x00\x00\x01".to_vec(),
        ImageLimits {
            max_images: 4,
            max_encoded_bytes_each: 4 * 1024 * 1024,
            max_total_encoded_bytes: 12 * 1024 * 1024,
            max_dimension_px: 4_096,
            max_decoded_pixels_each: 8_847_360,
        },
    )
    .unwrap()
}

#[tokio::test]
async fn vision_uses_exact_k3_shape_header_only_secret_and_success_terminal() {
    let server = ProviderServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(header("authorization", format!("Bearer {KEY}")))
        .respond_with(sse_response(VISION_STREAM_OK))
        .expect(1)
        .mount(server.mock_server())
        .await;

    let events = KimiProvider::new_for_test(&format!("{}/v1", server.uri()))
        .unwrap()
        .stream_vision(
            &SecretString::from(KEY),
            visual_request("kimi-k3"),
            CancellationToken::new(),
        )
        .await
        .unwrap()
        .collect::<Vec<_>>()
        .await;
    assert_eq!(
        events,
        vec![
            Ok(UnifiedStreamEvent::TextDelta {
                text: "Visible Kimi vision answer.".to_owned(),
            }),
            Ok(UnifiedStreamEvent::Usage {
                input_tokens: Some(49),
                output_tokens: Some(10),
            }),
            Ok(UnifiedStreamEvent::Completed),
        ]
    );

    let received = server.mock_server().received_requests().await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&received[0].body).unwrap();
    assert_eq!(body["model"], "kimi-k3");
    assert_eq!(body["max_completion_tokens"], 321);
    assert_eq!(body["stream"], true);
    assert_eq!(body["stream_options"], json!({"include_usage":true}));
    assert_eq!(body["reasoning_effort"], "low");
    assert_eq!(
        body["messages"][0],
        json!({"role":"system","content":"fixture-visual-system"})
    );
    assert_eq!(
        body["messages"][1],
        json!({"role":"user","content":"fixture-prior-question"})
    );
    assert_eq!(
        body["messages"][2],
        json!({"role":"assistant","content":"fixture-prior-answer"})
    );
    assert_eq!(body["messages"][3]["role"], "user");
    assert_eq!(
        body["messages"][3]["content"][0],
        json!({"type":"image_url","image_url":{"url":format!("data:image/png;base64,{SYNTHETIC_PNG_BASE64}")}})
    );
    assert_eq!(
        body["messages"][3]["content"][1],
        json!({"type":"text","text":"fixture-visible-image-question"})
    );
    assert!(
        !String::from_utf8(received[0].body.clone())
            .unwrap()
            .contains(KEY)
    );
    assert_eq!(received.len(), 1, "inline image flow must not upload files");
}

#[tokio::test]
async fn structured_pages_require_stop_visible_json_then_bounded_decode() {
    let server = ProviderServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(header("authorization", format!("Bearer {KEY}")))
        .respond_with(ResponseTemplate::new(200).set_body_raw(STRUCTURED_OK, "application/json"))
        .expect(1)
        .mount(server.mock_server())
        .await;

    let result = KimiProvider::new_for_test(&format!("{}/v1", server.uri()))
        .unwrap()
        .analyze_pages(
            &SecretString::from(KEY),
            structured_request("kimi-k3"),
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(result.analysis.schema_version, PAGE_ANALYSIS_SCHEMA_VERSION);
    assert!(result.cleanup.is_none());
    assert_eq!(
        result.analysis.pages[0].blocks[0].plain_text,
        "synthetic visible page text"
    );

    let received = server.mock_server().received_requests().await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&received[0].body).unwrap();
    assert_eq!(body["model"], "kimi-k3");
    assert_eq!(body["stream"], false);
    assert_eq!(body["max_completion_tokens"], 4_096);
    assert_eq!(body["reasoning_effort"], "low");
    assert_eq!(body["response_format"]["type"], "json_schema");
    assert_eq!(
        body["response_format"]["json_schema"]["name"],
        "textbooklens_page_analysis"
    );
    assert_eq!(body["response_format"]["json_schema"]["strict"], true);
    assert_eq!(
        body["response_format"]["json_schema"]["schema"]["properties"]["schemaVersion"]["enum"][0],
        PAGE_ANALYSIS_SCHEMA_VERSION
    );
    assert_eq!(body["messages"][0]["content"][0]["type"], "image_url");
    assert_eq!(
        body["messages"][0]["content"][0]["image_url"]["url"],
        format!("data:image/png;base64,{SYNTHETIC_PNG_BASE64}")
    );
    assert_eq!(body["messages"][0]["content"][1]["type"], "text");
}

#[tokio::test]
async fn visual_unknown_model_denies_before_credential_validation_or_http() {
    let server = ProviderServer::start().await;
    let provider = KimiProvider::new_for_test(&format!("{}/v1", server.uri())).unwrap();
    let credential = SecretString::from("");
    let vision = provider
        .stream_vision(
            &credential,
            visual_request("unverified-kimi-model"),
            CancellationToken::new(),
        )
        .await
        .err()
        .unwrap();
    let structured = provider
        .analyze_pages(
            &credential,
            structured_request("unverified-kimi-model"),
            CancellationToken::new(),
        )
        .await
        .unwrap_err();
    assert_eq!(vision.stable_code(), "UNSUPPORTED_PROVIDER_CAPABILITY");
    assert_eq!(structured.stable_code(), "UNSUPPORTED_PROVIDER_CAPABILITY");
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
async fn vision_truncation_and_structured_failures_or_cancel_never_complete() {
    let server = ProviderServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(sse_response(VISION_STREAM_TRUNCATED))
        .expect(1)
        .mount(server.mock_server())
        .await;
    let events = KimiProvider::new_for_test(&format!("{}/v1", server.uri()))
        .unwrap()
        .stream_vision(
            &SecretString::from(KEY),
            visual_request("kimi-k3"),
            CancellationToken::new(),
        )
        .await
        .unwrap()
        .collect::<Vec<_>>()
        .await;
    assert!(events.iter().any(Result::is_err));
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, Ok(UnifiedStreamEvent::Completed)))
    );

    for fixture in [
        STRUCTURED_REFUSAL,
        STRUCTURED_TRUNCATED,
        r#"{"choices":[{"index":0,"finish_reason":"stop","message":{"content":"not-json"}}]}"#,
    ] {
        let failure_server = ProviderServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(fixture, "application/json"))
            .expect(1)
            .mount(failure_server.mock_server())
            .await;
        let error = KimiProvider::new_for_test(&format!("{}/v1", failure_server.uri()))
            .unwrap()
            .analyze_pages(
                &SecretString::from(KEY),
                structured_request("kimi-k3"),
                CancellationToken::new(),
            )
            .await
            .unwrap_err();
        let safe = format!("{error:?} {error}");
        assert!(!safe.contains("fixture-vendor-refusal-sentinel"));
        assert!(!safe.contains("not-json"));
    }

    let cancel_server = ProviderServer::start().await;
    let cancel = CancellationToken::new();
    cancel.cancel();
    let error = KimiProvider::new_for_test(&format!("{}/v1", cancel_server.uri()))
        .unwrap()
        .stream_vision(&SecretString::from(KEY), visual_request("kimi-k3"), cancel)
        .await
        .err()
        .unwrap();
    assert_eq!(error.code, AppErrorCode::ImportCancelled);
    assert!(
        cancel_server
            .mock_server()
            .received_requests()
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn validates_exact_model_membership() {
    let server = ProviderServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .and(header("authorization", format!("Bearer {KEY}")))
        .respond_with(ResponseTemplate::new(200).set_body_raw(MODELS, "application/json"))
        .mount(server.mock_server())
        .await;
    let provider = KimiProvider::new_for_test(&format!("{}/v1", server.uri())).unwrap();
    assert!(
        provider
            .validate(&SecretString::from(KEY), "fixture-kimi-model")
            .await
            .is_ok()
    );
    assert_eq!(
        provider
            .validate(&SecretString::from(KEY), "absent")
            .await
            .unwrap_err()
            .kind(),
        AiErrorKind::ModelNotFound
    );
}

#[tokio::test]
async fn sends_k3_safe_shape_and_discards_reasoning() {
    let server = ProviderServer::start().await;
    Mock::given(method("POST")).and(path("/v1/chat/completions")).and(header("authorization", format!("Bearer {KEY}"))).and(body_json(json!({"model":"fixture-kimi-model","messages":[{"role":"system","content":"fixture-system"},{"role":"user","content":"fixture question"},{"role":"assistant","content":"prior answer"}],"max_completion_tokens":321,"stream":true,"stream_options":{"include_usage":true},"reasoning_effort":"low"}))).respond_with(ResponseTemplate::new(200).insert_header("content-type", "text/event-stream").set_body_raw(STREAM, "text/event-stream")).mount(server.mock_server()).await;
    let events = KimiProvider::new_for_test(&format!("{}/v1", server.uri()))
        .unwrap()
        .stream_chat(
            &SecretString::from(KEY),
            request(),
            CancellationToken::new(),
        )
        .await
        .unwrap()
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(
        events
            .iter()
            .filter_map(|event| match event {
                UnifiedStreamEvent::TextDelta { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<String>(),
        "你好"
    );
    assert!(events.contains(&UnifiedStreamEvent::Usage {
        input_tokens: Some(7),
        output_tokens: Some(3)
    }));
    assert!(matches!(events.last(), Some(UnifiedStreamEvent::Completed)));
}

#[tokio::test]
async fn unicode_split_and_error_or_early_done_never_emit_bad_completion() {
    let bytes = STREAM.as_bytes();
    let split = bytes
        .windows(3)
        .position(|part| part == "你".as_bytes())
        .unwrap()
        + 1;
    let server =
        ChunkedSseServer::start(vec![bytes[..split].to_vec(), bytes[split..].to_vec()]).await;
    let events = KimiProvider::new_for_test(&format!("{}/v1", server.uri()))
        .unwrap()
        .stream_chat(
            &SecretString::from(KEY),
            request(),
            CancellationToken::new(),
        )
        .await
        .unwrap()
        .collect::<Vec<_>>()
        .await;
    assert!(events.iter().all(Result::is_ok));
    for body in [ERROR, "data: [DONE]\n\n"] {
        let server = ProviderServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_raw(body, "text/event-stream"),
            )
            .mount(server.mock_server())
            .await;
        let events = KimiProvider::new_for_test(&format!("{}/v1", server.uri()))
            .unwrap()
            .stream_chat(
                &SecretString::from(KEY),
                request(),
                CancellationToken::new(),
            )
            .await
            .unwrap()
            .collect::<Vec<_>>()
            .await;
        assert!(events.iter().any(Result::is_err));
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, Ok(UnifiedStreamEvent::Completed)))
        );
    }
}
