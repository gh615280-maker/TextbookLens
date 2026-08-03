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
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::Notify,
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;
use tracing_subscriber::fmt::MakeWriter;
use wiremock::{
    Mock, ResponseTemplate,
    matchers::{method, path},
};

use super::{
    error::{AiError, AiErrorKind},
    provider::{
        AiProvider, ProviderStream, UnifiedChatRequest, UnifiedMessage, UnifiedRole,
        UnifiedStreamEvent,
    },
    providers::{
        anthropic::AnthropicProvider, deepseek::DeepSeekProvider, gemini::GeminiProvider,
        kimi::KimiProvider, openai::OpenAiProvider,
    },
};
use crate::{
    domain::ProviderKind,
    errors::{AppErrorCode, AppErrorDto},
};
use provider_server::{ProviderServer, TEST_CREDENTIAL};

const OPENAI_VALIDATE: &str = include_str!("../../../fixtures/providers/openai/validate-ok.json");
const OPENAI_STREAM: &str = include_str!("../../../fixtures/providers/openai/stream-ok.sse");
const OPENAI_ERROR: &str = include_str!("../../../fixtures/providers/openai/stream-error.sse");
const GEMINI_VALIDATE: &str = include_str!("../../../fixtures/providers/gemini/validate-ok.json");
const GEMINI_STREAM: &str = include_str!("../../../fixtures/providers/gemini/stream-ok.sse");
const GEMINI_ERROR: &str = include_str!("../../../fixtures/providers/gemini/stream-error.sse");
const ANTHROPIC_VALIDATE: &str =
    include_str!("../../../fixtures/providers/anthropic/validate-ok.json");
const ANTHROPIC_STREAM: &str = include_str!("../../../fixtures/providers/anthropic/stream-ok.sse");
const ANTHROPIC_ERROR: &str =
    include_str!("../../../fixtures/providers/anthropic/stream-error.sse");
const DEEPSEEK_VALIDATE: &str =
    include_str!("../../../fixtures/providers/deepseek/validate-ok.json");
const DEEPSEEK_STREAM: &str = include_str!("../../../fixtures/providers/deepseek/stream-ok.sse");
const DEEPSEEK_ERROR: &str = include_str!("../../../fixtures/providers/deepseek/stream-error.sse");
const KIMI_VALIDATE: &str = include_str!("../../../fixtures/providers/kimi/validate-ok.json");
const KIMI_STREAM: &str = include_str!("../../../fixtures/providers/kimi/stream-ok.sse");
const KIMI_ERROR: &str = include_str!("../../../fixtures/providers/kimi/stream-error.sse");

const WAIT_LIMIT: Duration = Duration::from_secs(5);
const HIDDEN_SENTINEL: &str = "fixture-contract-hidden-sentinel";
const LATE_SENTINEL: &str = "fixture-contract-post-terminal-sentinel";

#[derive(Clone, Copy, Debug)]
enum ContractProvider {
    OpenAi,
    Gemini,
    Anthropic,
    DeepSeek,
    Kimi,
}

const PROVIDERS: [ContractProvider; 5] = [
    ContractProvider::OpenAi,
    ContractProvider::Gemini,
    ContractProvider::Anthropic,
    ContractProvider::DeepSeek,
    ContractProvider::Kimi,
];

impl ContractProvider {
    fn kind(self) -> ProviderKind {
        match self {
            Self::OpenAi => ProviderKind::OpenAi,
            Self::Gemini => ProviderKind::Gemini,
            Self::Anthropic => ProviderKind::Anthropic,
            Self::DeepSeek => ProviderKind::DeepSeek,
            Self::Kimi => ProviderKind::Kimi,
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

    fn model(self) -> &'static str {
        match self {
            Self::OpenAi => "fixture-openai/model?edition#1",
            Self::Gemini => "fixture-gemini/model?edition#1",
            Self::Anthropic => "fixture/model",
            Self::DeepSeek => "fixture-deepseek-model",
            Self::Kimi => "fixture-kimi-model",
        }
    }

    fn validation_path(self) -> &'static str {
        match self {
            Self::OpenAi => "/v1/models/fixture-openai%2Fmodel%3Fedition%231",
            Self::Gemini => "/v1beta/models/fixture-gemini%2Fmodel%3Fedition%231",
            Self::Anthropic => "/v1/models/fixture%2Fmodel",
            Self::DeepSeek => "/models",
            Self::Kimi => "/v1/models",
        }
    }

    fn validation_fixture(self) -> &'static str {
        match self {
            Self::OpenAi => OPENAI_VALIDATE,
            Self::Gemini => GEMINI_VALIDATE,
            Self::Anthropic => ANTHROPIC_VALIDATE,
            Self::DeepSeek => DEEPSEEK_VALIDATE,
            Self::Kimi => KIMI_VALIDATE,
        }
    }

    fn stream_path(self) -> &'static str {
        match self {
            Self::OpenAi => "/v1/responses",
            Self::Gemini => {
                "/v1beta/models/fixture-gemini%2Fmodel%3Fedition%231:streamGenerateContent"
            }
            Self::Anthropic => "/v1/messages",
            Self::DeepSeek => "/chat/completions",
            Self::Kimi => "/v1/chat/completions",
        }
    }

    fn stream_fixture(self) -> &'static str {
        match self {
            Self::OpenAi => OPENAI_STREAM,
            Self::Gemini => GEMINI_STREAM,
            Self::Anthropic => ANTHROPIC_STREAM,
            Self::DeepSeek => DEEPSEEK_STREAM,
            Self::Kimi => KIMI_STREAM,
        }
    }

    fn error_fixture(self) -> &'static str {
        match self {
            Self::OpenAi => OPENAI_ERROR,
            Self::Gemini => GEMINI_ERROR,
            Self::Anthropic => ANTHROPIC_ERROR,
            Self::DeepSeek => DEEPSEEK_ERROR,
            Self::Kimi => KIMI_ERROR,
        }
    }

    fn expected_text(self) -> &'static [&'static str] {
        match self {
            Self::Kimi => &["你", "好"],
            _ => &["Fixture ", "answer."],
        }
    }

    fn expected_usage_events(self) -> usize {
        match self {
            Self::Anthropic => 2,
            _ => 1,
        }
    }

    fn success_body(self) -> String {
        let with_hidden = match self {
            Self::OpenAi => self.stream_fixture().replacen(
                "event: response.completed",
                &format!(
                    "event: response.reasoning.delta\ndata: {{\"type\":\"response.reasoning.delta\",\"delta\":\"{HIDDEN_SENTINEL}\"}}\n\nevent: response.completed"
                ),
                1,
            ),
            Self::Anthropic => self.stream_fixture().replacen(
                "event: content_block_start",
                &format!(
                    "event: content_block_start\ndata: {{\"content_block\":{{\"type\":\"thinking\"}}}}\n\nevent: content_block_delta\ndata: {{\"delta\":{{\"type\":\"thinking_delta\",\"text\":\"{HIDDEN_SENTINEL}\"}}}}\n\nevent: content_block_delta\ndata: {{\"delta\":{{\"type\":\"signature_delta\",\"text\":\"{HIDDEN_SENTINEL}\"}}}}\n\nevent: content_block_stop\ndata: {{}}\n\nevent: content_block_start"
                ),
                1,
            ),
            _ => self.stream_fixture().to_owned(),
        };
        format!("{with_hidden}{}", self.post_terminal_frame())
    }

    fn post_terminal_frame(self) -> String {
        match self {
            Self::OpenAi => format!(
                "event: response.output_text.delta\ndata: {{\"type\":\"response.output_text.delta\",\"delta\":\"{LATE_SENTINEL}\"}}\n\n"
            ),
            Self::Gemini => format!(
                "data: {{\"candidates\":[{{\"index\":0,\"content\":{{\"parts\":[{{\"text\":\"{LATE_SENTINEL}\"}}]}}}}]}}\n\n"
            ),
            Self::Anthropic => format!(
                "event: content_block_delta\ndata: {{\"delta\":{{\"type\":\"text_delta\",\"text\":\"{LATE_SENTINEL}\"}}}}\n\n"
            ),
            Self::DeepSeek | Self::Kimi => format!(
                "data: {{\"choices\":[{{\"index\":0,\"delta\":{{\"content\":\"{LATE_SENTINEL}\"}},\"finish_reason\":null}}]}}\n\n"
            ),
        }
    }

    fn first_delta_frame(self) -> String {
        match self {
            Self::OpenAi => "event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"first\"}\n\n".to_owned(),
            Self::Gemini => "data: {\"candidates\":[{\"index\":0,\"content\":{\"parts\":[{\"text\":\"first\"}]}}]}\n\n".to_owned(),
            Self::Anthropic => concat!(
                "event: message_start\n",
                "data: {\"message\":{\"type\":\"message\",\"usage\":{\"input_tokens\":1}}}\n\n",
                "event: content_block_start\n",
                "data: {\"content_block\":{\"type\":\"text\"}}\n\n",
                "event: content_block_delta\n",
                "data: {\"delta\":{\"type\":\"text_delta\",\"text\":\"first\"}}\n\n"
            )
            .to_owned(),
            Self::DeepSeek | Self::Kimi => "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"first\"},\"finish_reason\":null}]}\n\n".to_owned(),
        }
    }

    fn terminal_frames(self) -> String {
        match self {
            Self::OpenAi => "event: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\",\"usage\":{\"input_tokens\":1,\"output_tokens\":1}}}\n\n".to_owned(),
            Self::Gemini => "data: {\"candidates\":[{\"index\":0,\"content\":{\"parts\":[{\"text\":\"terminal\"}]},\"finishReason\":\"STOP\"}],\"usageMetadata\":{\"promptTokenCount\":1,\"candidatesTokenCount\":1}}\n\n".to_owned(),
            Self::Anthropic => concat!(
                "event: content_block_stop\n",
                "data: {}\n\n",
                "event: message_delta\n",
                "data: {\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":1}}\n\n",
                "event: message_stop\n",
                "data: {}\n\n"
            )
            .to_owned(),
            Self::DeepSeek | Self::Kimi => concat!(
                "data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
                "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":1,\"completion_tokens\":1}}\n\n",
                "data: [DONE]\n\n"
            )
            .to_owned(),
        }
    }

    fn sensitive_stream(self) -> String {
        match self {
            Self::OpenAi => format!(
                "event: response.output_text.delta\ndata: {{\"type\":\"response.output_text.delta\",\"delta\":\"{VISIBLE_SENTINEL}\"}}\n\nevent: response.failed\ndata: {{\"type\":\"response.failed\",\"response\":{{\"status\":\"failed\",\"error\":{{\"code\":\"{VENDOR_SENTINEL}\",\"message\":\"{UNICODE_SENTINEL}\"}}}}}}\n\n"
            ),
            Self::Gemini => format!(
                "data: {{\"candidates\":[{{\"index\":0,\"content\":{{\"parts\":[{{\"text\":\"{VISIBLE_SENTINEL}\"}}]}}}}]}}\n\nevent: error\ndata: {{\"error\":{{\"status\":\"{VENDOR_SENTINEL}\",\"message\":\"{UNICODE_SENTINEL}\"}}}}\n\n"
            ),
            Self::Anthropic => format!(
                "event: message_start\ndata: {{\"message\":{{\"type\":\"message\",\"usage\":{{\"input_tokens\":1}}}}}}\n\nevent: content_block_start\ndata: {{\"content_block\":{{\"type\":\"text\"}}}}\n\nevent: content_block_delta\ndata: {{\"delta\":{{\"type\":\"text_delta\",\"text\":\"{VISIBLE_SENTINEL}\"}}}}\n\nevent: error\ndata: {{\"error\":{{\"type\":\"{VENDOR_SENTINEL}\",\"message\":\"{UNICODE_SENTINEL}\"}}}}\n\n"
            ),
            Self::DeepSeek | Self::Kimi => format!(
                "data: {{\"choices\":[{{\"index\":0,\"delta\":{{\"content\":\"{VISIBLE_SENTINEL}\"}},\"finish_reason\":null}}]}}\n\ndata: {{\"error\":{{\"code\":\"{VENDOR_SENTINEL}\",\"message\":\"{UNICODE_SENTINEL}\"}}}}\n\n"
            ),
        }
    }
}

fn request(case: ContractProvider) -> UnifiedChatRequest {
    UnifiedChatRequest {
        model: case.model().to_owned(),
        system: "fixture-system".to_owned(),
        messages: vec![UnifiedMessage {
            role: UnifiedRole::User,
            content: "fixture-question".to_owned(),
        }],
        max_output_tokens: 321,
        expected_language: Some("en".to_owned()),
    }
}

fn sse_response(body: String) -> ResponseTemplate {
    ResponseTemplate::new(200)
        .insert_header("content-type", "text/event-stream")
        .set_body_raw(body, "text/event-stream")
}

async fn collect_stream(
    case: ContractProvider,
    body: String,
    chat_request: UnifiedChatRequest,
) -> Vec<Result<UnifiedStreamEvent, AiError>> {
    let server = ProviderServer::start().await;
    Mock::given(method("POST"))
        .and(path(case.stream_path()))
        .respond_with(sse_response(body))
        .expect(1)
        .mount(server.mock_server())
        .await;
    let provider = case.provider(&server.uri());
    match provider
        .stream_chat(
            &SecretString::from(TEST_CREDENTIAL),
            chat_request,
            CancellationToken::new(),
        )
        .await
    {
        Ok(stream) => stream.collect().await,
        Err(error) => vec![Err(error)],
    }
}

#[tokio::test]
async fn provider_contract_validation_uses_each_provider_exact_operation() {
    for case in PROVIDERS {
        let server = ProviderServer::start().await;
        Mock::given(method("GET"))
            .and(path(case.validation_path()))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_raw(case.validation_fixture(), "application/json"),
            )
            .expect(1)
            .mount(server.mock_server())
            .await;
        let provider = case.provider(&server.uri());
        assert_eq!(provider.kind(), case.kind(), "{case:?}");
        let validation = provider
            .validate(&SecretString::from(TEST_CREDENTIAL), case.model())
            .await
            .unwrap_or_else(|error| panic!("{case:?} validation failed: {error:?}"));
        assert_eq!(validation.model, case.model(), "{case:?}");
        assert!(validation.context_window_tokens > 0, "{case:?}");
    }
}

#[tokio::test]
async fn provider_contract_success_emits_only_visible_ordered_text_usage_and_one_completion() {
    for case in PROVIDERS {
        let events = collect_stream(case, case.success_body(), request(case)).await;
        assert!(events.iter().all(Result::is_ok), "{case:?}: {events:?}");
        let events = events.into_iter().map(Result::unwrap).collect::<Vec<_>>();
        let text = events
            .iter()
            .filter_map(|event| match event {
                UnifiedStreamEvent::TextDelta { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(text, case.expected_text(), "{case:?}");
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, UnifiedStreamEvent::Usage { .. }))
                .count(),
            case.expected_usage_events(),
            "{case:?}"
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, UnifiedStreamEvent::Completed))
                .count(),
            1,
            "{case:?}"
        );
        assert!(
            matches!(events.last(), Some(UnifiedStreamEvent::Completed)),
            "{case:?}: {events:?}"
        );
        let serialized = serde_json::to_string(&events).unwrap();
        for hidden in [
            HIDDEN_SENTINEL,
            LATE_SENTINEL,
            "fixture-hidden-thought",
            "fixture-hidden-encrypted",
            "fixture-hidden-signature",
            "fixture-candidate-one-must-not-escape",
            "reasoning_content",
        ] {
            assert!(!serialized.contains(hidden), "{case:?} leaked {hidden}");
        }
    }
}

struct FailureVector {
    name: &'static str,
    body: String,
    expected: AiErrorKind,
}

fn failure_vectors(case: ContractProvider) -> Vec<FailureVector> {
    let unavailable = AiErrorKind::ProviderUnavailable;
    match case {
        ContractProvider::OpenAi => vec![
            FailureVector {
                name: "empty visible output",
                body: "event: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\",\"usage\":{\"input_tokens\":1,\"output_tokens\":0}}}\n\n".to_owned(),
                expected: unavailable,
            },
            FailureVector {
                name: "malformed known event",
                body: "event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"delta\":7}\n\n".to_owned(),
                expected: unavailable,
            },
            FailureVector {
                name: "vendor error",
                body: case.error_fixture().to_owned(),
                expected: unavailable,
            },
            FailureVector {
                name: "bad finish",
                body: "event: response.incomplete\ndata: {\"type\":\"response.incomplete\"}\n\n".to_owned(),
                expected: unavailable,
            },
            FailureVector {
                name: "usage only",
                body: "event: response.usage\ndata: {\"usage\":{\"input_tokens\":1,\"output_tokens\":1}}\n\n".to_owned(),
                expected: unavailable,
            },
            FailureVector {
                name: "done only",
                body: "data: [DONE]\n\n".to_owned(),
                expected: unavailable,
            },
            FailureVector {
                name: "EOF before terminal",
                body: "event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"partial\"}\n\n".to_owned(),
                expected: unavailable,
            },
        ],
        ContractProvider::Gemini => vec![
            FailureVector {
                name: "empty visible output",
                body: "data: {\"candidates\":[{\"index\":0,\"content\":{\"parts\":[{\"text\":\"hidden\",\"thought\":true}]},\"finishReason\":\"STOP\"}],\"usageMetadata\":{\"promptTokenCount\":1,\"candidatesTokenCount\":0}}\n\n".to_owned(),
                expected: unavailable,
            },
            FailureVector {
                name: "malformed known event",
                body: "data: {\"candidates\":{}}\n\n".to_owned(),
                expected: unavailable,
            },
            FailureVector {
                name: "vendor error",
                body: case.error_fixture().to_owned(),
                expected: unavailable,
            },
            FailureVector {
                name: "refusal finish",
                body: "data: {\"candidates\":[{\"index\":0,\"content\":{\"parts\":[{\"text\":\"blocked\"}]},\"finishReason\":\"SAFETY\"}]}\n\n".to_owned(),
                expected: AiErrorKind::ProviderRefused,
            },
            FailureVector {
                name: "usage only",
                body: "data: {\"usageMetadata\":{\"promptTokenCount\":1,\"candidatesTokenCount\":1}}\n\n".to_owned(),
                expected: unavailable,
            },
            FailureVector {
                name: "done only",
                body: "data: [DONE]\n\n".to_owned(),
                expected: unavailable,
            },
            FailureVector {
                name: "EOF before terminal",
                body: "data: {\"candidates\":[{\"index\":0,\"content\":{\"parts\":[{\"text\":\"partial\"}]}}]}\n\n".to_owned(),
                expected: unavailable,
            },
        ],
        ContractProvider::Anthropic => vec![
            FailureVector {
                name: "empty visible output",
                body: concat!(
                    "event: message_start\ndata: {\"message\":{\"type\":\"message\",\"usage\":{\"input_tokens\":1}}}\n\n",
                    "event: content_block_start\ndata: {\"content_block\":{\"type\":\"text\"}}\n\n",
                    "event: content_block_stop\ndata: {}\n\n",
                    "event: message_delta\ndata: {\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":0}}\n\n",
                    "event: message_stop\ndata: {}\n\n"
                ).to_owned(),
                expected: unavailable,
            },
            FailureVector {
                name: "malformed known event",
                body: "event: content_block_delta\ndata: {\"delta\":{\"type\":\"text_delta\",\"text\":\"orphan\"}}\n\n".to_owned(),
                expected: unavailable,
            },
            FailureVector {
                name: "vendor error",
                body: case.error_fixture().to_owned(),
                expected: unavailable,
            },
            FailureVector {
                name: "bad finish",
                body: concat!(
                    "event: message_start\ndata: {\"message\":{\"type\":\"message\",\"usage\":{\"input_tokens\":1}}}\n\n",
                    "event: content_block_start\ndata: {\"content_block\":{\"type\":\"text\"}}\n\n",
                    "event: content_block_delta\ndata: {\"delta\":{\"type\":\"text_delta\",\"text\":\"partial\"}}\n\n",
                    "event: content_block_stop\ndata: {}\n\n",
                    "event: message_delta\ndata: {\"delta\":{\"stop_reason\":\"max_tokens\"},\"usage\":{\"output_tokens\":1}}\n\n"
                ).to_owned(),
                expected: unavailable,
            },
            FailureVector {
                name: "usage only",
                body: "event: message_start\ndata: {\"message\":{\"type\":\"message\",\"usage\":{\"input_tokens\":1}}}\n\n".to_owned(),
                expected: unavailable,
            },
            FailureVector {
                name: "done only",
                body: "data: [DONE]\n\n".to_owned(),
                expected: unavailable,
            },
            FailureVector {
                name: "EOF before terminal",
                body: concat!(
                    "event: message_start\ndata: {\"message\":{\"type\":\"message\",\"usage\":{\"input_tokens\":1}}}\n\n",
                    "event: content_block_start\ndata: {\"content_block\":{\"type\":\"text\"}}\n\n",
                    "event: content_block_delta\ndata: {\"delta\":{\"type\":\"text_delta\",\"text\":\"partial\"}}\n\n",
                    "event: content_block_stop\ndata: {}\n\n",
                    "event: message_delta\ndata: {\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":1}}\n\n"
                ).to_owned(),
                expected: unavailable,
            },
        ],
        ContractProvider::DeepSeek | ContractProvider::Kimi => vec![
            FailureVector {
                name: "empty visible output",
                body: concat!(
                    "data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
                    "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":1,\"completion_tokens\":0}}\n\n",
                    "data: [DONE]\n\n"
                ).to_owned(),
                expected: unavailable,
            },
            FailureVector {
                name: "malformed known event",
                body: "data: {\"choices\":[{\"index\":0,\"finish_reason\":null}]}\n\n".to_owned(),
                expected: unavailable,
            },
            FailureVector {
                name: "vendor error",
                body: case.error_fixture().to_owned(),
                expected: unavailable,
            },
            FailureVector {
                name: "bad finish",
                body: "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"partial\"},\"finish_reason\":\"length\"}]}\n\n".to_owned(),
                expected: unavailable,
            },
            FailureVector {
                name: "usage only",
                body: "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":1,\"completion_tokens\":1}}\n\n".to_owned(),
                expected: unavailable,
            },
            FailureVector {
                name: "done only",
                body: "data: [DONE]\n\n".to_owned(),
                expected: unavailable,
            },
            FailureVector {
                name: "EOF before terminal",
                body: "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"partial\"},\"finish_reason\":null}]}\n\n".to_owned(),
                expected: unavailable,
            },
            FailureVector {
                name: "safe finish before required DONE",
                body: "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"partial\"},\"finish_reason\":\"stop\"}]}\n\n".to_owned(),
                expected: unavailable,
            },
        ],
    }
}

#[tokio::test]
async fn provider_contract_rejects_every_applicable_no_completion_vector() {
    for case in PROVIDERS {
        for vector in failure_vectors(case) {
            let events = collect_stream(case, vector.body, request(case)).await;
            assert!(
                !events
                    .iter()
                    .any(|event| matches!(event, Ok(UnifiedStreamEvent::Completed))),
                "{case:?} {} completed: {events:?}",
                vector.name
            );
            let error = events
                .iter()
                .find_map(|event| event.as_ref().err())
                .unwrap_or_else(|| panic!("{case:?} {} did not fail: {events:?}", vector.name));
            assert_eq!(error.kind(), vector.expected, "{case:?} {}", vector.name);
            assert_eq!(
                events.iter().filter(|event| event.is_err()).count(),
                1,
                "{case:?} {}: {events:?}",
                vector.name
            );
        }
    }
}

struct HeadersGateServer {
    uri: String,
    request_received: Arc<Notify>,
    allow_headers: Arc<Notify>,
    task: JoinHandle<()>,
}

impl HeadersGateServer {
    async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let request_received = Arc::new(Notify::new());
        let allow_headers = Arc::new(Notify::new());
        let received = request_received.clone();
        let release = allow_headers.clone();
        let task = tokio::spawn(async move {
            if let Ok((mut socket, _)) = listener.accept().await
                && read_http_request(&mut socket).await.is_ok()
            {
                received.notify_one();
                release.notified().await;
                let _ = socket
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\r\n")
                    .await;
            }
        });
        Self {
            uri: format!("http://{address}"),
            request_received,
            allow_headers,
            task,
        }
    }
}

impl Drop for HeadersGateServer {
    fn drop(&mut self) {
        self.allow_headers.notify_one();
        self.task.abort();
    }
}

struct StreamGateServer {
    uri: String,
    first_written: Arc<Notify>,
    allow_terminal: Arc<Notify>,
    terminal_written: Arc<Notify>,
    task: JoinHandle<()>,
}

impl StreamGateServer {
    async fn start(first: String, terminal: String) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let first_written = Arc::new(Notify::new());
        let allow_terminal = Arc::new(Notify::new());
        let terminal_written = Arc::new(Notify::new());
        let first_signal = first_written.clone();
        let terminal_gate = allow_terminal.clone();
        let terminal_signal = terminal_written.clone();
        let task = tokio::spawn(async move {
            if let Ok((mut socket, _)) = listener.accept().await
                && read_http_request(&mut socket).await.is_ok()
                && socket
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n",
                    )
                    .await
                    .is_ok()
                && write_http_chunk(&mut socket, first.as_bytes()).await.is_ok()
            {
                first_signal.notify_one();
                terminal_gate.notified().await;
                let _ = write_http_chunk(&mut socket, terminal.as_bytes()).await;
                let _ = socket.write_all(b"0\r\n\r\n").await;
                let _ = socket.flush().await;
                terminal_signal.notify_one();
            }
        });
        Self {
            uri: format!("http://{address}"),
            first_written,
            allow_terminal,
            terminal_written,
            task,
        }
    }
}

impl Drop for StreamGateServer {
    fn drop(&mut self) {
        self.allow_terminal.notify_one();
        self.task.abort();
    }
}

async fn read_http_request(socket: &mut TcpStream) -> io::Result<()> {
    let mut request = Vec::new();
    let mut buffer = [0_u8; 4096];
    let mut required = None;
    loop {
        let read = socket.read(&mut buffer).await?;
        if read == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "request ended before its declared body",
            ));
        }
        request.extend_from_slice(&buffer[..read]);
        if request.len() > 128 * 1024 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "test request exceeded bound",
            ));
        }
        if required.is_none()
            && let Some(header_end) = request.windows(4).position(|window| window == b"\r\n\r\n")
        {
            let header_end = header_end + 4;
            let headers = String::from_utf8_lossy(&request[..header_end]);
            let content_length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())
                        .flatten()
                })
                .unwrap_or(0);
            required = Some(header_end + content_length);
        }
        if required.is_some_and(|required| request.len() >= required) {
            return Ok(());
        }
    }
}

async fn write_http_chunk(socket: &mut TcpStream, bytes: &[u8]) -> io::Result<()> {
    socket
        .write_all(format!("{:X}\r\n", bytes.len()).as_bytes())
        .await?;
    socket.write_all(bytes).await?;
    socket.write_all(b"\r\n").await?;
    socket.flush().await
}

async fn start_gated_stream(
    case: ContractProvider,
    server: &StreamGateServer,
    cancel: CancellationToken,
) -> ProviderStream {
    let provider = case.provider(&server.uri);
    tokio::time::timeout(
        WAIT_LIMIT,
        provider.stream_chat(&SecretString::from(TEST_CREDENTIAL), request(case), cancel),
    )
    .await
    .unwrap_or_else(|_| panic!("{case:?} timed out waiting for stream headers"))
    .unwrap_or_else(|error| panic!("{case:?} failed before stream: {error:?}"))
}

async fn expect_first_text(case: ContractProvider, stream: &mut ProviderStream) {
    loop {
        let event = tokio::time::timeout(WAIT_LIMIT, stream.next())
            .await
            .unwrap_or_else(|_| panic!("{case:?} timed out waiting for first delta"))
            .unwrap_or_else(|| panic!("{case:?} ended before first delta"))
            .unwrap_or_else(|error| panic!("{case:?} failed before first delta: {error:?}"));
        match event {
            UnifiedStreamEvent::TextDelta { text } => {
                assert_eq!(text, "first", "{case:?}");
                return;
            }
            UnifiedStreamEvent::Usage { .. } => {}
            UnifiedStreamEvent::Completed => panic!("{case:?} completed before first delta"),
        }
    }
}

async fn expect_cancel_then_end(case: ContractProvider, stream: &mut ProviderStream) {
    let error = tokio::time::timeout(WAIT_LIMIT, stream.next())
        .await
        .unwrap_or_else(|_| panic!("{case:?} cancellation timed out"))
        .unwrap_or_else(|| panic!("{case:?} ended without cancellation error"))
        .unwrap_err();
    assert_eq!(error.kind(), AiErrorKind::Cancelled, "{case:?}");
    assert_eq!(error.app_code(), AppErrorCode::ImportCancelled, "{case:?}");
    assert!(
        tokio::time::timeout(WAIT_LIMIT, stream.next())
            .await
            .unwrap_or_else(|_| panic!("{case:?} did not terminate after cancellation"))
            .is_none(),
        "{case:?} emitted an event after cancellation"
    );
}

#[tokio::test]
async fn provider_contract_cancel_before_headers_returns_no_stream_events() {
    for case in PROVIDERS {
        let server = HeadersGateServer::start().await;
        let provider = case.provider(&server.uri);
        let credential = SecretString::from(TEST_CREDENTIAL);
        let cancel = CancellationToken::new();
        let operation = provider.stream_chat(&credential, request(case), cancel.clone());
        tokio::pin!(operation);

        tokio::select! {
            result = &mut operation => panic!("{case:?} returned before gated headers: {}", result.is_ok()),
            result = tokio::time::timeout(WAIT_LIMIT, server.request_received.notified()) => {
                result.unwrap_or_else(|_| panic!("{case:?} request never reached loopback server"));
            }
        }
        cancel.cancel();
        let error = tokio::time::timeout(WAIT_LIMIT, &mut operation)
            .await
            .unwrap_or_else(|_| panic!("{case:?} did not cancel before headers"))
            .err()
            .unwrap_or_else(|| panic!("{case:?} returned a stream after pre-header cancellation"));
        assert_eq!(error.kind(), AiErrorKind::Cancelled, "{case:?}");
    }
}

#[tokio::test]
async fn provider_contract_cancel_after_first_delta_drops_later_terminal_events() {
    for case in PROVIDERS {
        let server =
            StreamGateServer::start(case.first_delta_frame(), case.terminal_frames()).await;
        let cancel = CancellationToken::new();
        let mut stream = start_gated_stream(case, &server, cancel.clone()).await;
        tokio::time::timeout(WAIT_LIMIT, server.first_written.notified())
            .await
            .unwrap_or_else(|_| panic!("{case:?} first frame was not written"));
        expect_first_text(case, &mut stream).await;
        cancel.cancel();
        expect_cancel_then_end(case, &mut stream).await;
        server.allow_terminal.notify_one();
    }
}

#[tokio::test]
async fn provider_contract_cancel_after_terminal_parse_discards_queued_events_and_completion() {
    for case in PROVIDERS {
        let server =
            StreamGateServer::start(case.first_delta_frame(), case.terminal_frames()).await;
        let cancel = CancellationToken::new();
        let mut stream = start_gated_stream(case, &server, cancel.clone()).await;
        expect_first_text(case, &mut stream).await;
        server.allow_terminal.notify_one();
        tokio::time::timeout(WAIT_LIMIT, server.terminal_written.notified())
            .await
            .unwrap_or_else(|_| panic!("{case:?} terminal frames were not written"));

        let terminal_event = tokio::time::timeout(WAIT_LIMIT, stream.next())
            .await
            .unwrap_or_else(|_| panic!("{case:?} terminal parse timed out"))
            .unwrap_or_else(|| panic!("{case:?} ended before exposing terminal-adjacent event"))
            .unwrap_or_else(|error| panic!("{case:?} terminal parse failed: {error:?}"));
        assert!(
            !matches!(terminal_event, UnifiedStreamEvent::Completed),
            "{case:?} did not expose a cancellable local-consumption boundary"
        );
        cancel.cancel();
        expect_cancel_then_end(case, &mut stream).await;
    }
}

struct HttpErrorVector {
    name: &'static str,
    status: u16,
    body: &'static str,
    expected: AiErrorKind,
}

fn http_error_vectors(case: ContractProvider) -> Vec<HttpErrorVector> {
    vec![
        HttpErrorVector {
            name: "credential",
            status: 401,
            body: r#"{"error":{"type":"authentication_error"}}"#,
            expected: AiErrorKind::InvalidApiKey,
        },
        HttpErrorVector {
            name: "model missing",
            status: 404,
            body: r#"{"error":{"code":"model_not_found"}}"#,
            expected: AiErrorKind::ModelNotFound,
        },
        HttpErrorVector {
            name: "permission",
            status: 403,
            body: r#"{"error":{"code":"permission_denied"}}"#,
            expected: AiErrorKind::ProviderPermissionDenied,
        },
        HttpErrorVector {
            name: "rate",
            status: 429,
            body: r#"{"error":{"type":"rate_limit_error"}}"#,
            expected: AiErrorKind::RateLimited,
        },
        HttpErrorVector {
            name: "quota",
            status: 429,
            body: r#"{"error":{"code":"insufficient_quota"}}"#,
            expected: AiErrorKind::InsufficientQuota,
        },
        HttpErrorVector {
            name: "context",
            status: 400,
            body: r#"{"error":{"code":"context_length_exceeded"}}"#,
            expected: AiErrorKind::ContextTooLarge,
        },
        HttpErrorVector {
            name: "refusal",
            status: 400,
            body: r#"{"error":{"status":"safety_rejection"}}"#,
            expected: AiErrorKind::ProviderRefused,
        },
        HttpErrorVector {
            name: "5xx",
            status: 503,
            body: r#"{"error":{"code":"fixture-nested-provider-body","message":"供应商错误"}}"#,
            expected: AiErrorKind::ProviderUnavailable,
        },
        HttpErrorVector {
            name: "ambiguous 429",
            status: 429,
            body: r#"{"error":{"status":"RESOURCE_EXHAUSTED"}}"#,
            expected: if matches!(case, ContractProvider::Gemini) {
                AiErrorKind::ProviderUnavailable
            } else {
                AiErrorKind::RateLimited
            },
        },
    ]
}

async fn provider_http_error(case: ContractProvider, status: u16, body: &'static str) -> AiError {
    let server = ProviderServer::start().await;
    Mock::given(method("POST"))
        .and(path(case.stream_path()))
        .respond_with(ResponseTemplate::new(status).set_body_raw(body, "application/json"))
        .expect(1)
        .mount(server.mock_server())
        .await;
    let provider = case.provider(&server.uri());
    match provider
        .stream_chat(
            &SecretString::from(TEST_CREDENTIAL),
            request(case),
            CancellationToken::new(),
        )
        .await
    {
        Err(error) => error,
        Ok(stream) => stream
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .find_map(Result::err)
            .unwrap_or_else(|| panic!("{case:?} HTTP {status} did not fail")),
    }
}

#[tokio::test]
async fn provider_contract_normalizes_only_supported_http_and_network_error_facts() {
    for case in PROVIDERS {
        for vector in http_error_vectors(case) {
            let error = provider_http_error(case, vector.status, vector.body).await;
            assert_eq!(error.kind(), vector.expected, "{case:?} {}", vector.name);
        }

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let provider = case.provider(&format!("http://{address}"));
        let result = provider
            .stream_chat(
                &SecretString::from(TEST_CREDENTIAL),
                request(case),
                CancellationToken::new(),
            )
            .await;
        let error = match result {
            Err(error) => error,
            Ok(_) => panic!("{case:?} offline loopback unexpectedly returned a stream"),
        };
        assert_eq!(error.kind(), AiErrorKind::NetworkOffline, "{case:?}");
    }
}

const KEY_SENTINEL: &str = "fixture-contract-key-sentinel";
const SYSTEM_SENTINEL: &str = "fixture-contract-system-sentinel";
const USER_SENTINEL: &str = "fixture-contract-user-sentinel";
const ASSISTANT_SENTINEL: &str = "fixture-contract-assistant-sentinel";
const VISIBLE_SENTINEL: &str = "fixture-contract-visible-delta-sentinel";
const VENDOR_SENTINEL: &str = "fixture-contract-vendor-nested-sentinel";
const UNICODE_SENTINEL: &str = "fixture-contract-unicode-错误-センチネル";
const SENSITIVE_SENTINELS: [&str; 7] = [
    KEY_SENTINEL,
    SYSTEM_SENTINEL,
    USER_SENTINEL,
    ASSISTANT_SENTINEL,
    VISIBLE_SENTINEL,
    VENDOR_SENTINEL,
    UNICODE_SENTINEL,
];

fn sensitive_request(case: ContractProvider) -> UnifiedChatRequest {
    UnifiedChatRequest {
        model: case.model().to_owned(),
        system: SYSTEM_SENTINEL.to_owned(),
        messages: vec![
            UnifiedMessage {
                role: UnifiedRole::User,
                content: USER_SENTINEL.to_owned(),
            },
            UnifiedMessage {
                role: UnifiedRole::Assistant,
                content: ASSISTANT_SENTINEL.to_owned(),
            },
            UnifiedMessage {
                role: UnifiedRole::User,
                content: "final fixture question".to_owned(),
            },
        ],
        max_output_tokens: 32,
        expected_language: Some("en".to_owned()),
    }
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

fn safe_error_surfaces(error: AiError) -> Vec<String> {
    let app_error = error.into_app_error();
    let app_debug = format!("{app_error:?}");
    let app_display = app_error.to_string();
    let capture = LogCapture::default();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_writer(capture.clone())
        .finish();
    let dto = tracing::subscriber::with_default(subscriber, || AppErrorDto::from(app_error));
    vec![
        format!("{error:?}"),
        error.to_string(),
        std::error::Error::source(&error)
            .map(ToString::to_string)
            .unwrap_or_default(),
        format!("{:?}", io::Error::other(error)),
        app_debug,
        app_display,
        serde_json::to_string(&dto).unwrap(),
        capture.text(),
    ]
}

#[tokio::test]
async fn provider_contract_sensitive_inputs_never_enter_error_dto_source_or_tracing() {
    for case in PROVIDERS {
        let server = ProviderServer::start().await;
        Mock::given(method("POST"))
            .and(path(case.stream_path()))
            .respond_with(sse_response(case.sensitive_stream()))
            .expect(1)
            .mount(server.mock_server())
            .await;
        let provider = case.provider(&server.uri());
        let events = provider
            .stream_chat(
                &SecretString::from(KEY_SENTINEL),
                sensitive_request(case),
                CancellationToken::new(),
            )
            .await
            .unwrap_or_else(|error| panic!("{case:?} failed before visible probe: {error:?}"))
            .collect::<Vec<_>>()
            .await;
        assert!(
            events.iter().any(|event| matches!(
                event,
                Ok(UnifiedStreamEvent::TextDelta { text }) if text == VISIBLE_SENTINEL
            )),
            "{case:?}: {events:?}"
        );
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, Ok(UnifiedStreamEvent::Completed))),
            "{case:?}: {events:?}"
        );
        let error = events
            .into_iter()
            .find_map(Result::err)
            .unwrap_or_else(|| panic!("{case:?} sensitive probe did not fail"));
        for surface in safe_error_surfaces(error) {
            for sentinel in SENSITIVE_SENTINELS {
                assert!(
                    !surface.contains(sentinel),
                    "{case:?} leaked {sentinel} through a safe error surface"
                );
            }
        }
    }
}
