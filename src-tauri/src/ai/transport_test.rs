#[path = "../../tests/common/provider_server.rs"]
mod provider_server;

use std::{panic, time::Duration};

use futures_util::StreamExt;
use reqwest::{Method, StatusCode};
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::json;
use tokio_util::sync::CancellationToken;
use wiremock::{
    Mock, ResponseTemplate,
    matchers::{method, path},
};

use super::{
    error::{AiError, AiErrorKind},
    provider::{
        MAX_CREDENTIAL_BYTES, UnifiedChatRequest, UnifiedMessage, UnifiedRole, UnifiedStreamEvent,
        validate_credential, validate_display_name, validate_model_id,
    },
    stream::{SseEvent, SseEventMapper, parse_known_json},
    transport::{
        CredentialHeader, ProviderHttpRequest, ProviderTransport, TransportPolicy,
        zeroize_credential_buffer_for_test,
    },
};
use crate::{
    domain::ProviderKind,
    errors::{AppErrorCode, AppErrorDto},
};
use provider_server::{ChunkedSseServer, ProviderServer, TEST_CREDENTIAL};

struct BridgeMapper;

impl SseEventMapper for BridgeMapper {
    fn map_event(&mut self, event: &SseEvent) -> Result<Vec<UnifiedStreamEvent>, AiError> {
        #[derive(Deserialize)]
        struct Payload {
            text: Option<String>,
            status: Option<String>,
        }

        let payload: Payload = parse_known_json(event)?;
        match event.event_type() {
            "delta" => Ok(vec![UnifiedStreamEvent::TextDelta {
                text: payload.text.ok_or_else(AiError::malformed_event)?,
            }]),
            "complete" if payload.status.as_deref() == Some("completed") => {
                Ok(vec![UnifiedStreamEvent::Completed])
            }
            _ => Err(AiError::malformed_event()),
        }
    }
}

#[test]
fn unified_request_contract_and_local_bounds_are_strict() {
    let request = UnifiedChatRequest {
        model: "test-model".to_owned(),
        system: "Answer from supplied context only.".to_owned(),
        messages: vec![UnifiedMessage {
            role: UnifiedRole::User,
            content: "fixture-selected-passage".to_owned(),
        }],
        max_output_tokens: 4096,
        expected_language: Some("zh-CN".to_owned()),
    };
    assert_eq!(
        serde_json::to_value(request).unwrap(),
        json!({
            "model": "test-model",
            "system": "Answer from supplied context only.",
            "messages": [{"role": "user", "content": "fixture-selected-passage"}],
            "maxOutputTokens": 4096,
            "expectedLanguage": "zh-CN"
        })
    );

    assert!(validate_model_id("model-1").is_ok());
    assert!(validate_model_id("").is_err());
    assert!(validate_model_id(&"m".repeat(257)).is_err());
    assert!(validate_display_name("供应商").is_ok());
    assert!(validate_display_name(&"名".repeat(81)).is_err());
    assert!(validate_credential(&SecretString::from("x")).is_ok());
    assert!(
        validate_credential(&SecretString::from("x".repeat(MAX_CREDENTIAL_BYTES + 1))).is_err()
    );

    let credential = SecretString::from("fixture-unicode-凭据");
    assert!(zeroize_credential_buffer_for_test(&credential));
    let mut sensitive_request = ProviderHttpRequest::json(
        Method::POST,
        "generate",
        CredentialHeader::Bearer,
        &json!({"input": "fixture-owned-request-buffer"}),
    )
    .unwrap();
    assert!(sensitive_request.zeroize_body_for_test());
}

#[test]
fn production_origins_and_policy_are_fixed_and_hardened() {
    let expected = [
        (ProviderKind::OpenAi, "https://api.openai.com/"),
        (
            ProviderKind::Gemini,
            "https://generativelanguage.googleapis.com/",
        ),
        (ProviderKind::Anthropic, "https://api.anthropic.com/"),
        (ProviderKind::DeepSeek, "https://api.deepseek.com/"),
        (ProviderKind::Kimi, "https://api.moonshot.ai/v1/"),
    ];
    for (kind, origin) in expected {
        let transport = ProviderTransport::new(kind).unwrap();
        assert_eq!(transport.origin_for_test(), origin);
        assert!(transport.origin_for_test().starts_with("https://"));
    }

    assert_eq!(
        ProviderTransport::new(ProviderKind::OpenAi)
            .unwrap()
            .policy(),
        TransportPolicy {
            connect_timeout: Duration::from_secs(30),
            request_idle_timeout: Duration::from_secs(120),
            max_response_bytes: 1024 * 1024,
            max_sse_data_bytes: 1024 * 1024,
        }
    );
}

#[test]
fn test_http_origin_must_be_loopback_and_cannot_escape_its_base_path() {
    assert!(
        ProviderTransport::new_for_test(ProviderKind::OpenAi, "http://127.0.0.1:32123").is_ok()
    );
    assert!(ProviderTransport::new_for_test(ProviderKind::OpenAi, "http://[::1]:32123").is_ok());
    assert!(ProviderTransport::new_for_test(ProviderKind::OpenAi, "http://example.com").is_err());
    assert!(
        ProviderTransport::new_for_test(ProviderKind::OpenAi, "http://localhost.example.com")
            .is_err()
    );

    let transport =
        ProviderTransport::new_for_test(ProviderKind::Kimi, "http://127.0.0.1:32123/v1").unwrap();
    assert!(transport.resolve_endpoint_for_test("models").is_ok());
    assert!(transport.resolve_endpoint_for_test("../outside").is_err());
    assert!(
        transport
            .resolve_endpoint_for_test("//not-loopback.invalid/path")
            .is_err()
    );
}

#[tokio::test]
async fn bounded_request_injects_secret_only_at_send_and_advertises_compression() {
    let server = ProviderServer::start().await;
    Mock::given(method("POST"))
        .and(path("/generate"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true})))
        .expect(1)
        .mount(server.mock_server())
        .await;

    let transport = ProviderTransport::new_for_test(ProviderKind::OpenAi, &server.uri()).unwrap();
    let request = ProviderHttpRequest::json(
        Method::POST,
        "generate",
        CredentialHeader::Bearer,
        &json!({"input": "fixture-selected-passage", "sentinel": "fixture-body-sentinel"}),
    )
    .unwrap();
    let request_debug = format!("{request:?}");
    assert!(!request_debug.contains("fixture-selected-passage"));
    assert!(!request_debug.contains("fixture-body-sentinel"));

    let credential = SecretString::from(TEST_CREDENTIAL);
    let response = transport
        .send_bounded(request, &credential, CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.json::<serde_json::Value>().unwrap(),
        json!({"ok": true})
    );

    let received = server.mock_server().received_requests().await.unwrap();
    let sent = &received[0];
    let expected_authorization = format!("Bearer {TEST_CREDENTIAL}");
    assert_eq!(
        sent.headers
            .get("authorization")
            .and_then(|value| value.to_str().ok()),
        Some(expected_authorization.as_str())
    );
    let accept_encoding = sent
        .headers
        .get("accept-encoding")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(accept_encoding.contains("gzip"));
    assert!(accept_encoding.contains("br"));
    let user_agent = sent.headers.get("user-agent").unwrap().to_str().unwrap();
    assert!(user_agent.starts_with("TextbookLens/"));
    assert!(!user_agent.contains(TEST_CREDENTIAL));
    assert!(!sent.url.as_str().contains(TEST_CREDENTIAL));
}

#[tokio::test]
async fn redirects_and_automatic_retries_are_disabled() {
    let server = ProviderServer::start().await;
    Mock::given(method("GET"))
        .and(path("/redirect"))
        .respond_with(ResponseTemplate::new(307).insert_header("location", "/target"))
        .expect(1)
        .mount(server.mock_server())
        .await;
    Mock::given(method("GET"))
        .and(path("/target"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(server.mock_server())
        .await;
    Mock::given(method("GET"))
        .and(path("/unavailable"))
        .respond_with(ResponseTemplate::new(503))
        .expect(1)
        .mount(server.mock_server())
        .await;

    let transport = ProviderTransport::new_for_test(ProviderKind::OpenAi, &server.uri()).unwrap();
    let credential = SecretString::from(TEST_CREDENTIAL);
    for endpoint in ["redirect", "unavailable"] {
        let error = transport
            .send_bounded(
                ProviderHttpRequest::empty(Method::GET, endpoint, CredentialHeader::Bearer)
                    .unwrap(),
                &credential,
                CancellationToken::new(),
            )
            .await
            .unwrap_err();
        assert_eq!(error.kind(), AiErrorKind::ProviderUnavailable);
    }
}

#[tokio::test]
async fn response_limit_timeout_and_pre_send_cancellation_are_safe_failures() {
    let server = ProviderServer::start().await;
    Mock::given(method("GET"))
        .and(path("/oversized"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![b'x'; 1024 * 1024 + 1]))
        .mount(server.mock_server())
        .await;
    Mock::given(method("GET"))
        .and(path("/slow"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_millis(100)))
        .mount(server.mock_server())
        .await;
    Mock::given(method("GET"))
        .and(path("/cancelled"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(server.mock_server())
        .await;

    let credential = SecretString::from(TEST_CREDENTIAL);
    let transport = ProviderTransport::new_for_test(ProviderKind::OpenAi, &server.uri()).unwrap();
    let oversized = transport
        .send_bounded(
            ProviderHttpRequest::empty(Method::GET, "oversized", CredentialHeader::Bearer).unwrap(),
            &credential,
            CancellationToken::new(),
        )
        .await
        .unwrap_err();
    assert_eq!(oversized.kind(), AiErrorKind::ProviderUnavailable);

    let short = ProviderTransport::new_for_test_with_timeout(
        ProviderKind::OpenAi,
        &server.uri(),
        Duration::from_millis(20),
    )
    .unwrap();
    let timeout = short
        .send_bounded(
            ProviderHttpRequest::empty(Method::GET, "slow", CredentialHeader::Bearer).unwrap(),
            &credential,
            CancellationToken::new(),
        )
        .await
        .unwrap_err();
    assert_eq!(timeout.kind(), AiErrorKind::ProviderUnavailable);

    let cancel = CancellationToken::new();
    cancel.cancel();
    let cancelled = transport
        .send_bounded(
            ProviderHttpRequest::empty(Method::GET, "cancelled", CredentialHeader::Bearer).unwrap(),
            &credential,
            cancel,
        )
        .await
        .unwrap_err();
    assert_eq!(cancelled.kind(), AiErrorKind::Cancelled);
    assert_eq!(cancelled.app_code(), AppErrorCode::ImportCancelled);
}

#[tokio::test]
async fn loopback_connection_failure_normalizes_to_network_offline() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);

    let transport =
        ProviderTransport::new_for_test(ProviderKind::OpenAi, &format!("http://{address}"))
            .unwrap();
    let error = transport
        .send_bounded(
            ProviderHttpRequest::empty(Method::GET, "offline", CredentialHeader::Bearer).unwrap(),
            &SecretString::from(TEST_CREDENTIAL),
            CancellationToken::new(),
        )
        .await
        .unwrap_err();
    assert_eq!(error.kind(), AiErrorKind::NetworkOffline);
    assert_eq!(error.app_code(), AppErrorCode::NetworkOffline);
}

#[tokio::test]
async fn send_stream_preserves_utf8_split_across_loopback_http_chunks() {
    let bytes = "event: delta\ndata: {\"text\":\"跨块\"}\n\nevent: complete\ndata: {\"status\":\"completed\"}\n\n"
        .as_bytes();
    let split = bytes
        .windows(3)
        .position(|window| window == "跨".as_bytes())
        .unwrap()
        + 1;
    let server =
        ChunkedSseServer::start(vec![bytes[..split].to_vec(), bytes[split..].to_vec()]).await;
    let transport = ProviderTransport::new_for_test(ProviderKind::OpenAi, server.uri()).unwrap();
    let events = transport
        .send_stream(
            ProviderHttpRequest::empty(Method::GET, "stream", CredentialHeader::Bearer).unwrap(),
            &SecretString::from(TEST_CREDENTIAL),
            CancellationToken::new(),
        )
        .await
        .unwrap()
        .decode(BridgeMapper)
        .collect::<Vec<_>>()
        .await;

    assert_eq!(
        events,
        vec![
            Ok(UnifiedStreamEvent::TextDelta {
                text: "跨块".to_owned(),
            }),
            Ok(UnifiedStreamEvent::Completed),
        ]
    );
}

#[test]
fn vendor_errors_normalize_without_retaining_untrusted_bodies() {
    let cases = [
        (
            401,
            r#"{"error":{"type":"authentication_error"}}"#,
            AppErrorCode::InvalidApiKey,
        ),
        (
            404,
            r#"{"error":{"code":"model_not_found"}}"#,
            AppErrorCode::ModelNotFound,
        ),
        (
            403,
            r#"{"error":{"code":"permission_denied"}}"#,
            AppErrorCode::ProviderPermissionDenied,
        ),
        (
            403,
            r#"{"error":{"code":"region_restricted"}}"#,
            AppErrorCode::ProviderRegionRestricted,
        ),
        (
            429,
            r#"{"error":{"type":"rate_limit_error"}}"#,
            AppErrorCode::RateLimited,
        ),
        (
            429,
            r#"{"error":{"code":"insufficient_quota"}}"#,
            AppErrorCode::InsufficientQuota,
        ),
        (
            400,
            r#"{"error":{"code":"context_length_exceeded"}}"#,
            AppErrorCode::ContextTooLarge,
        ),
        (
            400,
            r#"{"error":{"status":"refused"}}"#,
            AppErrorCode::ProviderRefused,
        ),
        (
            503,
            r#"{"error":{"code":"fixture-response-body-sentinel"}}"#,
            AppErrorCode::ProviderUnavailable,
        ),
    ];

    for (status, body, expected) in cases {
        let error = AiError::from_http(StatusCode::from_u16(status).unwrap(), body.as_bytes());
        assert_eq!(error.app_code(), expected);
        assert!(!format!("{error:?}").contains(body));
        assert!(!error.to_string().contains(body));
    }

    let network = AiError::network_offline();
    assert_eq!(network.app_code(), AppErrorCode::NetworkOffline);

    let sentinel = "fixture-response-body-sentinel";
    let error = AiError::from_http(
        StatusCode::SERVICE_UNAVAILABLE,
        format!(r#"{{"error":{{"code":"{sentinel}"}}}}"#).as_bytes(),
    );
    let panic_result = panic::catch_unwind(|| panic!("{error:?}"));
    let panic_text = panic_result
        .unwrap_err()
        .downcast::<String>()
        .map(|value| *value)
        .unwrap();
    assert!(!panic_text.contains(sentinel));
    let chained = std::io::Error::other(error);
    assert!(!format!("{chained:?}").contains(sentinel));
    let dto = AppErrorDto::from(AiError::provider_unavailable().into_app_error());
    let dto_json = serde_json::to_string(&dto).unwrap();
    assert!(!dto_json.contains(sentinel));
}
