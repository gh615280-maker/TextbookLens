// This unit-test module reuses the loopback-only server fixture that the
// independently compiled transport tests also load.
#[allow(clippy::duplicate_mod)]
#[path = "../../../tests/common/provider_server.rs"]
mod provider_server;

use std::time::Duration;

use futures_util::StreamExt;
use secrecy::SecretString;
use serde_json::json;
use tokio_util::sync::CancellationToken;
use wiremock::{
    Mock, ResponseTemplate,
    matchers::{body_json, header, method, path},
};

use super::openai::OpenAiProvider;
use crate::{
    ai::{
        error::AiErrorKind,
        provider::{
            AiProvider, MAX_CREDENTIAL_BYTES, UnifiedChatRequest, UnifiedMessage, UnifiedRole,
            UnifiedStreamEvent,
        },
        registry::UNKNOWN_MODEL_CONTEXT_WINDOW_TOKENS,
    },
    domain::ProviderKind,
    errors::AppErrorCode,
};
use provider_server::{ChunkedSseServer, ProviderServer};

const TEST_KEY: &str = "test-openai-key-not-secret";
const BODY_SENTINEL: &str = "fixture-textbook-selection-sentinel";
const VALIDATE_FIXTURE: &str =
    include_str!("../../../../fixtures/providers/openai/validate-ok.json");
const STREAM_OK_FIXTURE: &str = include_str!("../../../../fixtures/providers/openai/stream-ok.sse");
const STREAM_ERROR_FIXTURE: &str =
    include_str!("../../../../fixtures/providers/openai/stream-error.sse");

fn request() -> UnifiedChatRequest {
    UnifiedChatRequest {
        model: "fixture-openai-model".to_owned(),
        system: "fixture-system-instruction".to_owned(),
        messages: vec![
            UnifiedMessage {
                role: UnifiedRole::User,
                content: "fixture-first-question".to_owned(),
            },
            UnifiedMessage {
                role: UnifiedRole::Assistant,
                content: "fixture-prior-visible-answer".to_owned(),
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

#[tokio::test]
async fn validation_percent_encodes_one_model_segment_and_maps_auth_and_missing_model() {
    let server = ProviderServer::start().await;
    let model = "fixture-openai/model?edition#1";
    let encoded_path = "/v1/models/fixture-openai%2Fmodel%3Fedition%231";
    Mock::given(method("GET"))
        .and(path(encoded_path))
        .and(header("authorization", format!("Bearer {TEST_KEY}")))
        .respond_with(ResponseTemplate::new(200).set_body_raw(VALIDATE_FIXTURE, "application/json"))
        .expect(1)
        .mount(server.mock_server())
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/models/fixture-invalid-key"))
        .respond_with(
            ResponseTemplate::new(401).set_body_json(json!({"error":{"type":"invalid_api_key"}})),
        )
        .expect(1)
        .mount(server.mock_server())
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/models/fixture-missing-model"))
        .respond_with(
            ResponseTemplate::new(404).set_body_json(json!({"error":{"code":"model_not_found"}})),
        )
        .expect(1)
        .mount(server.mock_server())
        .await;

    let provider = OpenAiProvider::new_for_test(&server.uri()).unwrap();
    assert_eq!(provider.kind(), ProviderKind::OpenAi);
    let credential = SecretString::from(TEST_KEY);
    let validated = provider.validate(&credential, model).await.unwrap();
    assert_eq!(validated.model, model);
    assert_eq!(
        validated.context_window_tokens,
        UNKNOWN_MODEL_CONTEXT_WINDOW_TOKENS
    );

    let invalid_key = provider
        .validate(&credential, "fixture-invalid-key")
        .await
        .unwrap_err();
    assert_eq!(invalid_key.kind(), AiErrorKind::InvalidApiKey);
    assert_eq!(invalid_key.app_code(), AppErrorCode::InvalidApiKey);
    let missing_model = provider
        .validate(&credential, "fixture-missing-model")
        .await
        .unwrap_err();
    assert_eq!(missing_model.kind(), AiErrorKind::ModelNotFound);
    assert_eq!(missing_model.app_code(), AppErrorCode::ModelNotFound);

    let received = server.mock_server().received_requests().await.unwrap();
    assert_eq!(received.len(), 3);
    assert!(received[0].url.as_str().ends_with(encoded_path));
    for sent in &received {
        assert_eq!(sent.method.as_str(), "GET");
        assert!(!sent.url.as_str().contains(TEST_KEY));
        assert!(!sent.url.as_str().contains(BODY_SENTINEL));
        for (name, value) in &sent.headers {
            if name.as_str() != "authorization" {
                let value = value.to_str().unwrap();
                assert!(!value.contains(TEST_KEY));
                assert!(!value.contains(BODY_SENTINEL));
            }
        }
    }
}

#[tokio::test]
async fn generation_sends_exact_responses_request_without_secret_or_body_leakage() {
    let server = ProviderServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .and(header("authorization", format!("Bearer {TEST_KEY}")))
        .and(header("content-type", "application/json"))
        .and(body_json(json!({
            "model": "fixture-openai-model",
            "stream": true,
            "instructions": "fixture-system-instruction",
            "input": [
                {"role": "user", "content": "fixture-first-question"},
                {"role": "assistant", "content": "fixture-prior-visible-answer"},
                {"role": "user", "content": BODY_SENTINEL}
            ],
            "max_output_tokens": 321,
            "text": {"format": {"type": "text"}}
        })))
        .respond_with(sse_response(STREAM_OK_FIXTURE))
        .expect(1)
        .mount(server.mock_server())
        .await;

    let provider = OpenAiProvider::new_for_test(&server.uri()).unwrap();
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
    assert_eq!(sent.method.as_str(), "POST");
    assert_eq!(sent.url.path(), "/v1/responses");
    assert!(!sent.url.as_str().contains(TEST_KEY));
    assert!(!sent.url.as_str().contains(BODY_SENTINEL));
    for (name, value) in &sent.headers {
        if name.as_str() != "authorization" {
            let value = value.to_str().unwrap();
            assert!(!value.contains(TEST_KEY));
            assert!(!value.contains(BODY_SENTINEL));
        }
    }
}

#[tokio::test]
async fn split_tcp_stream_emits_only_visible_text_one_usage_and_one_completion() {
    let bytes = STREAM_OK_FIXTURE.as_bytes();
    let split_a = bytes
        .windows(8)
        .position(|window| window == b"Fixture ")
        .unwrap()
        + 3;
    let split_b = bytes
        .windows(7)
        .position(|window| window == b"answer.")
        .unwrap()
        + 2;
    let server = ChunkedSseServer::start(vec![
        bytes[..split_a].to_vec(),
        bytes[split_a..split_b].to_vec(),
        bytes[split_b..].to_vec(),
    ])
    .await;
    let provider = OpenAiProvider::new_for_test(server.uri()).unwrap();
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

    let events = events.into_iter().collect::<Result<Vec<_>, _>>().unwrap();
    let joined = events
        .iter()
        .filter_map(|event| match event {
            UnifiedStreamEvent::TextDelta { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<String>();
    assert_eq!(joined, "Fixture answer.");
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, UnifiedStreamEvent::Usage { .. }))
            .count(),
        1
    );
    assert!(events.contains(&UnifiedStreamEvent::Usage {
        input_tokens: Some(7),
        output_tokens: Some(3),
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

async fn collect_fixture(
    body: &'static str,
) -> Vec<Result<UnifiedStreamEvent, crate::ai::error::AiError>> {
    let server = ChunkedSseServer::start(vec![body.as_bytes().to_vec()]).await;
    OpenAiProvider::new_for_test(server.uri())
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
async fn failed_error_malformed_and_missing_completion_are_safe_failures() {
    let cases = [
        STREAM_ERROR_FIXTURE,
        "event: error\ndata: {\"type\":\"error\",\"code\":\"server_error\",\"message\":\"Synthetic top-level failure.\",\"param\":null}\n\n",
        "event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"delta\":7}\n\n",
        "event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"partial\"}\n\n",
        "event: response.future.added\ndata: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\"}}\n\n",
    ];

    for body in cases {
        let events = collect_fixture(body).await;
        assert!(
            events.iter().any(Result::is_err),
            "fixture should fail: {body}"
        );
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, Ok(UnifiedStreamEvent::Completed)))
        );
        assert!(!format!("{events:?}").contains("Synthetic top-level failure."));
    }
}

#[tokio::test]
async fn rate_limit_and_quota_http_errors_use_the_shared_error_mapper() {
    for (body, expected) in [
        (
            json!({"error":{"type":"rate_limit_error"}}),
            AiErrorKind::RateLimited,
        ),
        (
            json!({"error":{"code":"insufficient_quota"}}),
            AiErrorKind::InsufficientQuota,
        ),
    ] {
        let server = ProviderServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/responses"))
            .respond_with(ResponseTemplate::new(429).set_body_json(body))
            .expect(1)
            .mount(server.mock_server())
            .await;
        let result = OpenAiProvider::new_for_test(&server.uri())
            .unwrap()
            .stream_chat(
                &SecretString::from(TEST_KEY),
                request(),
                CancellationToken::new(),
            )
            .await;
        let error = match result {
            Err(error) => error,
            Ok(_) => panic!("HTTP 429 must fail before a stream is returned"),
        };
        assert_eq!(error.kind(), expected);
    }
}

#[tokio::test]
async fn hidden_reasoning_is_discarded_and_unknown_additive_events_are_ignored() {
    let hidden = "fixture-hidden-reasoning-sentinel";
    let body = format!(
        "event: response.reasoning_summary_text.delta\ndata: {{\"type\":\"response.reasoning_summary_text.delta\",\"delta\":\"{hidden}\"}}\n\n\
         event: response.reasoning_text.delta\ndata: {{\"type\":\"response.reasoning_text.delta\",\"delta\":\"{hidden}\"}}\n\n\
         event: response.future.added\ndata: {{\"type\":\"response.future.added\",\"thought\":\"{hidden}\"}}\n\n\
         event: response.output_text.delta\ndata: {{\"type\":\"response.output_text.delta\",\"delta\":\"visible\"}}\n\n\
         event: response.completed\ndata: {{\"type\":\"response.completed\",\"response\":{{\"status\":\"completed\",\"usage\":null}}}}\n\n"
    );
    let server = ChunkedSseServer::start(vec![body.into_bytes()]).await;
    let events = OpenAiProvider::new_for_test(server.uri())
        .unwrap()
        .stream_chat(
            &SecretString::from(TEST_KEY),
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
        events,
        vec![
            UnifiedStreamEvent::TextDelta {
                text: "visible".to_owned()
            },
            UnifiedStreamEvent::Completed
        ]
    );
    let ipc = serde_json::to_string(&events).unwrap();
    assert!(!ipc.contains(hidden));
}

#[tokio::test]
async fn zero_output_cap_and_input_bounds_fail_before_send() {
    let server = ProviderServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(sse_response(STREAM_OK_FIXTURE))
        .expect(0)
        .mount(server.mock_server())
        .await;
    let provider = OpenAiProvider::new_for_test(&server.uri()).unwrap();
    let mut invalid = request();
    invalid.max_output_tokens = 0;
    let result = provider
        .stream_chat(
            &SecretString::from(TEST_KEY),
            invalid,
            CancellationToken::new(),
        )
        .await;
    let error = match result {
        Err(error) => error,
        Ok(_) => panic!("zero output cap must fail before a stream is returned"),
    };
    assert_eq!(error.kind(), AiErrorKind::InvalidInput);
    assert_eq!(
        provider
            .validate(
                &SecretString::from("x".repeat(MAX_CREDENTIAL_BYTES + 1)),
                "fixture-openai-model",
            )
            .await
            .unwrap_err()
            .kind(),
        AiErrorKind::InvalidInput
    );
    assert_eq!(
        provider
            .validate(&SecretString::from(TEST_KEY), &"m".repeat(257))
            .await
            .unwrap_err()
            .kind(),
        AiErrorKind::InvalidInput
    );
}

#[tokio::test]
async fn cancellation_after_first_delta_drops_stream_without_later_events() {
    let first = "event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"first\"}\n\n";
    let later = "event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"later\"}\n\nevent: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\",\"usage\":null}}\n\n";
    let server =
        ChunkedSseServer::start(vec![first.as_bytes().to_vec(), later.as_bytes().to_vec()]).await;
    let provider = OpenAiProvider::new_for_test(server.uri()).unwrap();
    let cancel = CancellationToken::new();
    let mut stream = provider
        .stream_chat(&SecretString::from(TEST_KEY), request(), cancel.clone())
        .await
        .unwrap();
    assert_eq!(
        stream.next().await.unwrap().unwrap(),
        UnifiedStreamEvent::TextDelta {
            text: "first".to_owned()
        }
    );
    cancel.cancel();
    assert_eq!(
        stream.next().await.unwrap().unwrap_err().kind(),
        AiErrorKind::Cancelled
    );
    assert!(stream.next().await.is_none());
    tokio::time::sleep(Duration::from_millis(10)).await;
}
