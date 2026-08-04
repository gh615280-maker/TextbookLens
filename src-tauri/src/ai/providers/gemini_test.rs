// This independently compiled module reuses the loopback-only HTTP/SSE fixture.
#[allow(clippy::duplicate_mod)]
#[path = "../../../tests/common/provider_server.rs"]
mod provider_server;

use std::time::Duration;

use futures_util::StreamExt;
use secrecy::SecretString;
use serde_json::json;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use wiremock::{
    Mock, ResponseTemplate,
    matchers::{body_json, header, method, path},
};

use super::gemini::GeminiProvider;
use crate::{
    ai::{
        error::AiErrorKind,
        multimodal::stage_vision_asset,
        provider::{
            AiProvider, UnifiedChatRequest, UnifiedMessage, UnifiedRole, UnifiedStreamEvent,
        },
        structured::PAGE_ANALYSIS_SCHEMA_VERSION,
    },
    domain::{ImageLimits, ImageMime, ProviderKind, StructuredPageRequest, UnifiedVisionRequest},
    errors::AppErrorCode,
};
use provider_server::{ChunkedSseServer, ProviderServer};

const TEST_KEY: &str = "test-gemini-key-not-secret";
const BODY_SENTINEL: &str = "fixture-textbook-selection-sentinel";
const VALIDATE_FIXTURE: &str =
    include_str!("../../../../fixtures/providers/gemini/validate-ok.json");
const STREAM_OK_FIXTURE: &str = include_str!("../../../../fixtures/providers/gemini/stream-ok.sse");
const STREAM_ERROR_FIXTURE: &str =
    include_str!("../../../../fixtures/providers/gemini/stream-error.sse");
const VISION_STREAM_OK_FIXTURE: &str =
    include_str!("../../../../fixtures/providers/gemini/vision-stream-ok.sse");
const VISION_STREAM_TRUNCATED_FIXTURE: &str =
    include_str!("../../../../fixtures/providers/gemini/vision-stream-truncated.sse");
const STRUCTURED_OK_FIXTURE: &str =
    include_str!("../../../../fixtures/providers/gemini/structured-ok.json");
const STRUCTURED_REFUSAL_FIXTURE: &str =
    include_str!("../../../../fixtures/providers/gemini/structured-refusal.json");
const STRUCTURED_TRUNCATED_FIXTURE: &str =
    include_str!("../../../../fixtures/providers/gemini/structured-truncated.json");
const SYNTHETIC_PNG_BASE64: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAAB";

fn request() -> UnifiedChatRequest {
    UnifiedChatRequest {
        model: "fixture-gemini-model".to_owned(),
        system: "fixture-system-instruction".to_owned(),
        messages: vec![
            UnifiedMessage {
                role: UnifiedRole::User,
                content: "fixture-first-question".to_owned(),
            },
            UnifiedMessage {
                role: UnifiedRole::Assistant,
                content: "fixture-prior-a".to_owned(),
            },
            UnifiedMessage {
                role: UnifiedRole::Assistant,
                content: "fixture-prior-b".to_owned(),
            },
            UnifiedMessage {
                role: UnifiedRole::User,
                content: BODY_SENTINEL.to_owned(),
            },
        ],
        max_output_tokens: 321,
        expected_language: Some("fixture-language".to_owned()),
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
        image_limits(),
    )
    .unwrap()
}

fn image_limits() -> ImageLimits {
    ImageLimits {
        max_images: 4,
        max_encoded_bytes_each: 4 * 1024 * 1024,
        max_total_encoded_bytes: 12 * 1024 * 1024,
        max_dimension_px: 4_096,
        max_decoded_pixels_each: 8_847_360,
    }
}

#[tokio::test]
async fn vision_uses_exact_inline_data_shape_header_only_key_and_stop_terminal() {
    let server = ProviderServer::start().await;
    Mock::given(method("POST"))
        .and(path(
            "/v1beta/models/gemini-3.6-flash:streamGenerateContent",
        ))
        .and(header("x-goog-api-key", TEST_KEY))
        .and(header("content-type", "application/json"))
        .respond_with(sse_response(VISION_STREAM_OK_FIXTURE))
        .expect(1)
        .mount(server.mock_server())
        .await;

    let events = GeminiProvider::new_for_test(&server.uri())
        .unwrap()
        .stream_vision(
            &SecretString::from(TEST_KEY),
            visual_request("gemini-3.6-flash"),
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
                text: "Visible Gemini vision answer.".to_owned()
            }),
            Ok(UnifiedStreamEvent::Usage {
                input_tokens: Some(43),
                output_tokens: Some(8)
            }),
            Ok(UnifiedStreamEvent::Completed)
        ]
    );

    let received = server.mock_server().received_requests().await.unwrap();
    let sent = &received[0];
    assert_eq!(sent.url.query(), Some("alt=sse"));
    let body: serde_json::Value = serde_json::from_slice(&sent.body).unwrap();
    assert_eq!(
        body["systemInstruction"],
        json!({"parts":[{"text":"fixture-visual-system"}]})
    );
    assert_eq!(body["generationConfig"], json!({"maxOutputTokens":321}));
    assert_eq!(
        body["contents"][0],
        json!({"role":"user","parts":[{"text":"fixture-prior-question"}]})
    );
    assert_eq!(
        body["contents"][1],
        json!({"role":"model","parts":[{"text":"fixture-prior-answer"}]})
    );
    assert_eq!(
        body["contents"][2],
        json!({
            "role":"user",
            "parts":[
                {"inlineData":{"mimeType":"image/png","data":SYNTHETIC_PNG_BASE64}},
                {"text":"fixture-visible-image-question"}
            ]
        })
    );
    assert!(
        !String::from_utf8(sent.body.clone())
            .unwrap()
            .contains(TEST_KEY)
    );
    assert_eq!(
        received.len(),
        1,
        "inline image flow must not create a Files resource"
    );
}

#[tokio::test]
async fn structured_pages_use_response_format_stop_then_bounded_decode() {
    let server = ProviderServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1beta/models/gemini-3.6-flash:generateContent"))
        .and(header("x-goog-api-key", TEST_KEY))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(STRUCTURED_OK_FIXTURE, "application/json"),
        )
        .expect(1)
        .mount(server.mock_server())
        .await;

    let result = GeminiProvider::new_for_test(&server.uri())
        .unwrap()
        .analyze_pages(
            &SecretString::from(TEST_KEY),
            structured_request("gemini-3.6-flash"),
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
    assert_eq!(body["generationConfig"]["maxOutputTokens"], 4_096);
    assert_eq!(
        body["generationConfig"]["responseFormat"]["text"]["mimeType"],
        "application/json"
    );
    assert_eq!(
        body["generationConfig"]["responseFormat"]["text"]["schema"]["properties"]["schemaVersion"]
            ["enum"][0],
        PAGE_ANALYSIS_SCHEMA_VERSION
    );
    assert_eq!(
        body["contents"][0]["parts"][0],
        json!({"inlineData":{"mimeType":"image/png","data":SYNTHETIC_PNG_BASE64}})
    );
    assert!(
        body["contents"][0]["parts"][1]["text"]
            .as_str()
            .unwrap()
            .contains(PAGE_ANALYSIS_SCHEMA_VERSION)
    );
    assert!(
        !String::from_utf8(received[0].body.clone())
            .unwrap()
            .contains(TEST_KEY)
    );
}

#[tokio::test]
async fn visual_unknown_model_denies_before_credential_validation_or_http() {
    let server = ProviderServer::start().await;
    let provider = GeminiProvider::new_for_test(&server.uri()).unwrap();
    let empty_credential = SecretString::from("");
    let vision = provider
        .stream_vision(
            &empty_credential,
            visual_request("unverified-gemini-model"),
            CancellationToken::new(),
        )
        .await
        .err()
        .unwrap();
    let structured = provider
        .analyze_pages(
            &empty_credential,
            structured_request("unverified-gemini-model"),
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
async fn vision_max_tokens_and_structured_refusal_truncation_or_malformed_never_complete() {
    let server = ProviderServer::start().await;
    Mock::given(method("POST"))
        .and(path(
            "/v1beta/models/gemini-3.6-flash:streamGenerateContent",
        ))
        .respond_with(sse_response(VISION_STREAM_TRUNCATED_FIXTURE))
        .expect(1)
        .mount(server.mock_server())
        .await;
    let events = GeminiProvider::new_for_test(&server.uri())
        .unwrap()
        .stream_vision(
            &SecretString::from(TEST_KEY),
            visual_request("gemini-3.6-flash"),
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
        STRUCTURED_REFUSAL_FIXTURE,
        STRUCTURED_TRUNCATED_FIXTURE,
        r#"{"candidates":[{"index":0,"content":{"parts":[{"text":"not-json"}]},"finishReason":"STOP"}]}"#,
    ] {
        let failure_server = ProviderServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1beta/models/gemini-3.6-flash:generateContent"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(fixture, "application/json"))
            .expect(1)
            .mount(failure_server.mock_server())
            .await;
        let error = GeminiProvider::new_for_test(&failure_server.uri())
            .unwrap()
            .analyze_pages(
                &SecretString::from(TEST_KEY),
                structured_request("gemini-3.6-flash"),
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
    let error = GeminiProvider::new_for_test(&cancel_server.uri())
        .unwrap()
        .stream_vision(
            &SecretString::from(TEST_KEY),
            visual_request("gemini-3.6-flash"),
            cancel,
        )
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
async fn validation_uses_encoded_resource_path_header_only_key_and_strict_metadata() {
    let server = ProviderServer::start().await;
    let model = "fixture-gemini/model?edition#1";
    let encoded_path = "/v1beta/models/fixture-gemini%2Fmodel%3Fedition%231";
    Mock::given(method("GET"))
        .and(path(encoded_path))
        .and(header("x-goog-api-key", TEST_KEY))
        .respond_with(ResponseTemplate::new(200).set_body_raw(VALIDATE_FIXTURE, "application/json"))
        .expect(1)
        .mount(server.mock_server())
        .await;
    Mock::given(method("GET"))
        .and(path("/v1beta/models/fixture-invalid-key"))
        .respond_with(
            ResponseTemplate::new(401).set_body_json(json!({"error":{"status":"UNAUTHENTICATED"}})),
        )
        .expect(1)
        .mount(server.mock_server())
        .await;
    Mock::given(method("GET"))
        .and(path("/v1beta/models/fixture-missing-model"))
        .respond_with(
            ResponseTemplate::new(404).set_body_json(json!({"error":{"status":"NOT_FOUND"}})),
        )
        .expect(1)
        .mount(server.mock_server())
        .await;
    Mock::given(method("GET"))
        .and(path("/v1beta/models/fixture-no-generation"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "name":"models/fixture-no-generation", "inputTokenLimit":1, "outputTokenLimit":1,
            "supportedGenerationMethods":["embedContent"]
        })))
        .expect(1)
        .mount(server.mock_server())
        .await;
    Mock::given(method("GET"))
        .and(path("/v1beta/models/fixture-malformed"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "name":"models/fixture-malformed", "inputTokenLimit":"not-a-number",
            "outputTokenLimit":1, "supportedGenerationMethods":["generateContent"]
        })))
        .expect(1)
        .mount(server.mock_server())
        .await;

    let provider = GeminiProvider::new_for_test(&server.uri()).unwrap();
    assert_eq!(provider.kind(), ProviderKind::Gemini);
    let credential = SecretString::from(TEST_KEY);
    let validated = provider.validate(&credential, model).await.unwrap();
    assert_eq!(validated.model, model);
    assert_eq!(validated.context_window_tokens, 321);
    assert_eq!(
        provider
            .validate(&credential, "fixture-invalid-key")
            .await
            .unwrap_err()
            .kind(),
        AiErrorKind::InvalidApiKey
    );
    assert_eq!(
        provider
            .validate(&credential, "fixture-missing-model")
            .await
            .unwrap_err()
            .kind(),
        AiErrorKind::ModelNotFound
    );
    for model in ["fixture-no-generation", "fixture-malformed"] {
        assert_eq!(
            provider
                .validate(&credential, model)
                .await
                .unwrap_err()
                .kind(),
            AiErrorKind::ProviderUnavailable
        );
    }

    let received = server.mock_server().received_requests().await.unwrap();
    assert_eq!(received.len(), 5);
    assert!(received[0].url.as_str().ends_with(encoded_path));
    for sent in &received {
        assert_eq!(sent.method.as_str(), "GET");
        assert!(sent.url.query().is_none());
        assert!(sent.body.is_empty());
        assert!(!sent.url.as_str().contains(TEST_KEY));
        assert!(sent.headers.get("authorization").is_none());
        assert_eq!(
            sent.headers
                .get("x-goog-api-key")
                .unwrap()
                .to_str()
                .unwrap(),
            TEST_KEY
        );
    }
}

#[tokio::test]
async fn generation_serializes_coalesced_gemini_history_with_header_only_key() {
    let server = ProviderServer::start().await;
    Mock::given(method("POST"))
        .and(path(
            "/v1beta/models/fixture-gemini-model:streamGenerateContent",
        ))
        .and(header("x-goog-api-key", TEST_KEY))
        .and(header("content-type", "application/json"))
        .and(body_json(json!({
            "systemInstruction":{"parts":[{"text":"fixture-system-instruction"}]},
            "contents":[
                {"role":"user","parts":[{"text":"fixture-first-question"}]},
                {"role":"model","parts":[{"text":"fixture-prior-a"},{"text":"fixture-prior-b"}]},
                {"role":"user","parts":[{"text":BODY_SENTINEL}]}
            ],
            "generationConfig":{"maxOutputTokens":321}
        })))
        .respond_with(sse_response(STREAM_OK_FIXTURE))
        .expect(1)
        .mount(server.mock_server())
        .await;

    let provider = GeminiProvider::new_for_test(&server.uri()).unwrap();
    let debug = format!("{provider:?}");
    assert!(!debug.contains(TEST_KEY));
    assert!(!debug.contains(BODY_SENTINEL));
    let events = provider
        .stream_chat(
            &SecretString::from(TEST_KEY),
            request(),
            CancellationToken::new(),
        )
        .await
        .unwrap()
        .collect::<Vec<_>>()
        .await;
    assert!(events.iter().all(Result::is_ok));
    let received = server.mock_server().received_requests().await.unwrap();
    let sent = &received[0];
    assert_eq!(sent.url.query(), Some("alt=sse"));
    assert!(!sent.url.as_str().contains(TEST_KEY));
    assert!(!sent.url.as_str().contains(BODY_SENTINEL));
    assert!(sent.headers.get("authorization").is_none());
    for (name, value) in &sent.headers {
        if name.as_str() != "x-goog-api-key" {
            let value = value.to_str().unwrap();
            assert!(!value.contains(TEST_KEY));
            assert!(!value.contains(BODY_SENTINEL));
        }
    }
}

#[tokio::test]
async fn local_history_and_output_cap_rejections_happen_before_send() {
    let server = ProviderServer::start().await;
    Mock::given(method("POST"))
        .respond_with(sse_response(STREAM_OK_FIXTURE))
        .expect(0)
        .mount(server.mock_server())
        .await;
    let provider = GeminiProvider::new_for_test(&server.uri()).unwrap();
    let invalid_requests = [
        UnifiedChatRequest {
            messages: vec![],
            ..request()
        },
        UnifiedChatRequest {
            messages: vec![UnifiedMessage {
                role: UnifiedRole::Assistant,
                content: "fixture-first".to_owned(),
            }],
            ..request()
        },
        UnifiedChatRequest {
            messages: vec![UnifiedMessage {
                role: UnifiedRole::User,
                content: "".to_owned(),
            }],
            ..request()
        },
        UnifiedChatRequest {
            max_output_tokens: 0,
            ..request()
        },
    ];
    for invalid in invalid_requests {
        let result = provider
            .stream_chat(
                &SecretString::from(TEST_KEY),
                invalid,
                CancellationToken::new(),
            )
            .await;
        assert!(matches!(result, Err(error) if error.kind() == AiErrorKind::InvalidInput));
    }
}

async fn collect_body(body: Vec<u8>) -> Vec<Result<UnifiedStreamEvent, crate::ai::error::AiError>> {
    let server = ChunkedSseServer::start(vec![body]).await;
    GeminiProvider::new_for_test(server.uri())
        .unwrap()
        .stream_chat(
            &SecretString::from(TEST_KEY),
            request(),
            CancellationToken::new(),
        )
        .await
        .unwrap()
        .collect()
        .await
}

#[tokio::test]
async fn split_stream_uses_only_candidate_zero_visible_text_terminal_usage_and_stop() {
    let bytes = STREAM_OK_FIXTURE.as_bytes();
    let split = bytes
        .windows(8)
        .position(|value| value == b"Fixture ")
        .unwrap()
        + 3;
    let server =
        ChunkedSseServer::start(vec![bytes[..split].to_vec(), bytes[split..].to_vec()]).await;
    let events = GeminiProvider::new_for_test(server.uri())
        .unwrap()
        .stream_chat(
            &SecretString::from(TEST_KEY),
            request(),
            CancellationToken::new(),
        )
        .await
        .unwrap()
        .collect::<Vec<_>>()
        .await;
    let events = events.into_iter().collect::<Result<Vec<_>, _>>().unwrap();
    let visible = events
        .iter()
        .filter_map(|event| match event {
            UnifiedStreamEvent::TextDelta { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<String>();
    assert_eq!(visible, "Fixture answer.");
    assert!(!visible.contains("hidden"));
    assert!(!visible.contains("candidate-one"));
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, UnifiedStreamEvent::Usage { .. }))
            .count(),
        1
    );
    assert!(events.contains(&UnifiedStreamEvent::Usage {
        input_tokens: Some(7),
        output_tokens: Some(3)
    }));
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, UnifiedStreamEvent::Completed))
            .count(),
        1
    );
    assert!(matches!(events.last(), Some(UnifiedStreamEvent::Completed)));
}

#[tokio::test]
async fn max_tokens_is_the_other_accepted_terminal_reason() {
    let body = b"data: {\"candidates\":[{\"index\":0,\"content\":{\"parts\":[{\"text\":\"visible\"}]},\"finishReason\":\"MAX_TOKENS\"}]}\n\n".to_vec();
    let events = collect_body(body).await;
    assert!(events.iter().all(Result::is_ok));
    assert!(matches!(
        events.last(),
        Some(Ok(UnifiedStreamEvent::Completed))
    ));
}

#[tokio::test]
async fn rejected_terminals_prompt_blocks_malformed_candidate_and_eof_fail_without_completion() {
    let mut cases = vec![
        b"data: {\"promptFeedback\":{\"blockReason\":\"SAFETY\"}}\n\n".to_vec(),
        b"data: {\"candidates\":[]}\n\n".to_vec(),
        b"data: {\"candidates\":[{\"index\":1,\"content\":{\"parts\":[{\"text\":\"wrong-candidate\"}]}}]}\n\n".to_vec(),
        b"data: {\"candidates\":[{\"index\":0,\"finishReason\":\"STOP\"}]}\n\n".to_vec(),
        b"data: {\"candidates\":[{\"index\":0,\"content\":{\"parts\":[{\"thoughtSignature\":\"hidden\"}]},\"finishReason\":\"STOP\"}]}\n\n".to_vec(),
        b"data: {\"candidates\":[{\"index\":0,\"content\":{\"parts\":[{\"text\":\"partial\"}]}}]}\n\n".to_vec(),
    ];
    for reason in [
        "SAFETY",
        "RECITATION",
        "LANGUAGE",
        "OTHER",
        "BLOCKLIST",
        "PROHIBITED_CONTENT",
        "SPII",
        "MALFORMED_FUNCTION_CALL",
        "IMAGE_SAFETY",
        "UNKNOWN_NEW_REASON",
    ] {
        cases.push(format!("data: {{\"candidates\":[{{\"index\":0,\"content\":{{\"parts\":[{{\"text\":\"must-not-escape\"}}]}},\"finishReason\":\"{reason}\"}}]}}\n\n").into_bytes());
    }
    for body in cases {
        let events = collect_body(body).await;
        assert!(events.iter().any(Result::is_err));
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, Ok(UnifiedStreamEvent::Completed)))
        );
    }
}

#[tokio::test]
async fn http_error_mapping_and_sse_error_robustness_are_safe() {
    for (status, body, expected) in [
        (
            403,
            json!({"error":{"status":"PERMISSION_DENIED"}}),
            AiErrorKind::ProviderPermissionDenied,
        ),
        (
            404,
            json!({"error":{"status":"NOT_FOUND"}}),
            AiErrorKind::ModelNotFound,
        ),
        (
            429,
            json!({"error":{"status":"RATE_LIMITED"}}),
            AiErrorKind::RateLimited,
        ),
        (
            429,
            json!({"error":{"status":"QUOTA_EXHAUSTED"}}),
            AiErrorKind::InsufficientQuota,
        ),
        (
            429,
            json!({"error":{"status":"RESOURCE_EXHAUSTED"}}),
            AiErrorKind::ProviderUnavailable,
        ),
    ] {
        let server = ProviderServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(status).set_body_json(body))
            .mount(server.mock_server())
            .await;
        let result = GeminiProvider::new_for_test(&server.uri())
            .unwrap()
            .stream_chat(
                &SecretString::from(TEST_KEY),
                request(),
                CancellationToken::new(),
            )
            .await;
        assert!(matches!(result, Err(error) if error.kind() == expected));
    }
    let events = collect_body(STREAM_ERROR_FIXTURE.as_bytes().to_vec()).await;
    assert!(events.iter().any(Result::is_err));
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, Ok(UnifiedStreamEvent::Completed)))
    );
    assert!(!format!("{events:?}").contains("Synthetic Gemini stream failure."));
}

#[tokio::test]
async fn cancellation_drops_later_delta_usage_and_completion() {
    let first = b"data: {\"candidates\":[{\"index\":0,\"content\":{\"parts\":[{\"text\":\"first\"}]}}]}\n\n".to_vec();
    let later = b"data: {\"candidates\":[{\"index\":0,\"content\":{\"parts\":[{\"text\":\"later\"}]},\"finishReason\":\"STOP\"}],\"usageMetadata\":{\"promptTokenCount\":1,\"candidatesTokenCount\":1}}\n\n".to_vec();
    let server = ChunkedSseServer::start(vec![first, later]).await;
    let cancel = CancellationToken::new();
    let mut stream = GeminiProvider::new_for_test(server.uri())
        .unwrap()
        .stream_chat(&SecretString::from(TEST_KEY), request(), cancel.clone())
        .await
        .unwrap();
    assert!(matches!(
        stream.next().await,
        Some(Ok(UnifiedStreamEvent::TextDelta { .. }))
    ));
    cancel.cancel();
    assert!(
        matches!(stream.next().await, Some(Err(error)) if error.kind() == AiErrorKind::Cancelled)
    );
    assert!(stream.next().await.is_none());
    tokio::time::sleep(Duration::from_millis(10)).await;
}
