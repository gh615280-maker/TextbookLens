#[allow(clippy::duplicate_mod)]
#[path = "../../../tests/common/provider_server.rs"]
mod provider_server;

use futures_util::StreamExt;
use secrecy::SecretString;
use serde_json::json;
use tokio_util::sync::CancellationToken;
use wiremock::{
    Mock, ResponseTemplate,
    matchers::{body_json, header, method, path},
};

use super::kimi::KimiProvider;
use crate::ai::{
    error::AiErrorKind,
    provider::{AiProvider, UnifiedChatRequest, UnifiedMessage, UnifiedRole, UnifiedStreamEvent},
};
use provider_server::{ChunkedSseServer, ProviderServer};

const KEY: &str = "test-kimi-key-not-secret";
const MODELS: &str = include_str!("../../../../fixtures/providers/kimi/validate-ok.json");
const STREAM: &str = include_str!("../../../../fixtures/providers/kimi/stream-ok.sse");
const ERROR: &str = include_str!("../../../../fixtures/providers/kimi/stream-error.sse");
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
