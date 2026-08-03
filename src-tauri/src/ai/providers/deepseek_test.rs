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

use super::deepseek::DeepSeekProvider;
use crate::ai::{
    error::AiErrorKind,
    multimodal::stage_vision_asset,
    provider::{AiProvider, UnifiedChatRequest, UnifiedMessage, UnifiedRole, UnifiedStreamEvent},
    structured::PAGE_ANALYSIS_SCHEMA_VERSION,
};
use crate::domain::{ImageLimits, ImageMime, StructuredPageRequest, UnifiedVisionRequest};
use provider_server::ProviderServer;

const KEY: &str = "test-deepseek-key-not-secret";
const MODELS: &str = include_str!("../../../../fixtures/providers/deepseek/validate-ok.json");
const STREAM: &str = include_str!("../../../../fixtures/providers/deepseek/stream-ok.sse");
const ERROR: &str = include_str!("../../../../fixtures/providers/deepseek/stream-error.sse");
fn request() -> UnifiedChatRequest {
    UnifiedChatRequest {
        model: "fixture-deepseek-model".into(),
        system: "ignored-system".into(),
        messages: vec![
            UnifiedMessage {
                role: UnifiedRole::Assistant,
                content: "prior answer".into(),
            },
            UnifiedMessage {
                role: UnifiedRole::User,
                content: "fixture question".into(),
            },
        ],
        max_output_tokens: 321,
        expected_language: None,
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
async fn visual_and_page_analysis_deny_locally_before_credential_or_http() {
    let server = ProviderServer::start().await;
    let provider = DeepSeekProvider::new_for_test(&server.uri()).unwrap();
    let credential = SecretString::from("");
    let book_id = Uuid::new_v4();
    let vision = UnifiedVisionRequest {
        text: UnifiedChatRequest {
            model: "deepseek-v4-flash".to_owned(),
            system: String::new(),
            messages: vec![UnifiedMessage {
                role: UnifiedRole::User,
                content: "fixture-visible-image-question".to_owned(),
            }],
            max_output_tokens: 321,
            expected_language: None,
        },
        images: vec![synthetic_image(book_id)],
    };
    let structured = StructuredPageRequest {
        model: "deepseek-v4-flash".to_owned(),
        pages: vec![synthetic_image(book_id)],
        schema_version: PAGE_ANALYSIS_SCHEMA_VERSION.to_owned(),
        max_output_bytes: 4_096,
    };

    let vision_error = provider
        .stream_vision(&credential, vision, CancellationToken::new())
        .await
        .err()
        .unwrap();
    let structured_error = provider
        .analyze_pages(&credential, structured, CancellationToken::new())
        .await
        .unwrap_err();
    assert_eq!(
        vision_error.stable_code(),
        "UNSUPPORTED_PROVIDER_CAPABILITY"
    );
    assert_eq!(
        structured_error.stable_code(),
        "UNSUPPORTED_PROVIDER_CAPABILITY"
    );
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
async fn validates_model_list_exact_membership() {
    let server = ProviderServer::start().await;
    Mock::given(method("GET"))
        .and(path("/models"))
        .and(header("authorization", format!("Bearer {KEY}")))
        .respond_with(ResponseTemplate::new(200).set_body_raw(MODELS, "application/json"))
        .mount(server.mock_server())
        .await;
    let provider = DeepSeekProvider::new_for_test(&server.uri()).unwrap();
    assert!(
        provider
            .validate(&SecretString::from(KEY), "fixture-deepseek-model")
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
async fn sends_current_official_stream_shape_and_requires_stop_done() {
    let server = ProviderServer::start().await;
    Mock::given(method("POST")).and(path("/chat/completions")).and(header("authorization", format!("Bearer {KEY}"))).and(body_json(json!({"model":"fixture-deepseek-model","messages":[{"role":"assistant","content":"prior answer"},{"role":"user","content":"fixture question"}],"thinking":{"type":"disabled"},"max_tokens":321,"stream":true,"stream_options":{"include_usage":true}}))).respond_with(ResponseTemplate::new(200).insert_header("content-type", "text/event-stream").set_body_raw(STREAM, "text/event-stream")).mount(server.mock_server()).await;
    let events = DeepSeekProvider::new_for_test(&server.uri())
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
        "Fixture answer."
    );
    assert!(events.contains(&UnifiedStreamEvent::Usage {
        input_tokens: Some(7),
        output_tokens: Some(3)
    }));
    assert!(matches!(events.last(), Some(UnifiedStreamEvent::Completed)));
}

#[tokio::test]
async fn rejects_error_and_done_without_stop() {
    for body in [
        ERROR,
        "data: [DONE]\n\n",
        "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"text\"},\"finish_reason\":\"length\"}]}\n\n",
    ] {
        let server = ProviderServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_raw(body, "text/event-stream"),
            )
            .mount(server.mock_server())
            .await;
        let events = DeepSeekProvider::new_for_test(&server.uri())
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
