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

use super::deepseek::DeepSeekProvider;
use crate::ai::{
    error::AiErrorKind,
    provider::{AiProvider, UnifiedChatRequest, UnifiedMessage, UnifiedRole, UnifiedStreamEvent},
};
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
