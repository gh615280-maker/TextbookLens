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

use super::anthropic::AnthropicProvider;
use crate::{
    ai::{
        error::AiErrorKind,
        provider::{
            AiProvider, UnifiedChatRequest, UnifiedMessage, UnifiedRole, UnifiedStreamEvent,
        },
    },
    domain::ProviderKind,
};
use provider_server::ProviderServer;

const KEY: &str = "test-anthropic-key-not-secret";
const VALIDATE: &str = include_str!("../../../../fixtures/providers/anthropic/validate-ok.json");
const STREAM: &str = include_str!("../../../../fixtures/providers/anthropic/stream-ok.sse");
const ERROR: &str = include_str!("../../../../fixtures/providers/anthropic/stream-error.sse");
fn request() -> UnifiedChatRequest {
    UnifiedChatRequest {
        model: "fixture-anthropic-model".into(),
        system: "fixture-system".into(),
        messages: vec![
            UnifiedMessage {
                role: UnifiedRole::User,
                content: "fixture-question".into(),
            },
            UnifiedMessage {
                role: UnifiedRole::Assistant,
                content: "fixture-answer".into(),
            },
            UnifiedMessage {
                role: UnifiedRole::User,
                content: "fixture-followup".into(),
            },
        ],
        max_output_tokens: 321,
        expected_language: None,
    }
}

#[tokio::test]
async fn validates_encoded_model_with_required_headers() {
    let server = ProviderServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models/fixture%2Fmodel"))
        .and(header("x-api-key", KEY))
        .and(header("anthropic-version", "2023-06-01"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(VALIDATE, "application/json"))
        .mount(server.mock_server())
        .await;
    let provider = AnthropicProvider::new_for_test(&server.uri()).unwrap();
    assert_eq!(provider.kind(), ProviderKind::Anthropic);
    assert_eq!(
        provider
            .validate(&SecretString::from(KEY), "fixture/model")
            .await
            .unwrap()
            .model,
        "fixture/model"
    );
}

#[tokio::test]
async fn sends_messages_shape_and_only_emits_text_usage_completion() {
    let server = ProviderServer::start().await;
    Mock::given(method("POST")).and(path("/v1/messages")).and(header("x-api-key", KEY)).and(header("anthropic-version", "2023-06-01")).and(body_json(json!({"model":"fixture-anthropic-model","system":"fixture-system","messages":[{"role":"user","content":"fixture-question"},{"role":"assistant","content":"fixture-answer"},{"role":"user","content":"fixture-followup"}],"max_tokens":321,"stream":true}))).respond_with(ResponseTemplate::new(200).insert_header("content-type", "text/event-stream").set_body_raw(STREAM, "text/event-stream")).mount(server.mock_server()).await;
    let events = AnthropicProvider::new_for_test(&server.uri())
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
        output_tokens: None
    }));
    assert!(events.contains(&UnifiedStreamEvent::Usage {
        input_tokens: None,
        output_tokens: Some(3)
    }));
    assert!(matches!(events.last(), Some(UnifiedStreamEvent::Completed)));
}

#[tokio::test]
async fn error_and_truncated_stream_never_complete() {
    for body in [
        ERROR,
        "event: message_start\ndata: {\"message\":{\"type\":\"message\",\"usage\":{\"input_tokens\":1}}}\n\n",
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
        let events = AnthropicProvider::new_for_test(&server.uri())
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
    let server = ProviderServer::start().await;
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(401)
                .set_body_json(json!({"error":{"type":"authentication_error"}})),
        )
        .mount(server.mock_server())
        .await;
    assert_eq!(
        AnthropicProvider::new_for_test(&server.uri())
            .unwrap()
            .validate(&SecretString::from(KEY), "bad")
            .await
            .unwrap_err()
            .kind(),
        AiErrorKind::InvalidApiKey
    );
}
