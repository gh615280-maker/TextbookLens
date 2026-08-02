use std::{
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll},
    time::Duration,
};

use futures_util::{Stream, StreamExt, stream};
use reqwest::StatusCode;
use serde::Deserialize;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::{
    error::{AiError, AiErrorKind},
    provider::UnifiedStreamEvent,
    stream::{MAX_SSE_DATA_BYTES, SseEvent, SseEventMapper, decode_sse, parse_known_json},
};
use crate::errors::AppErrorCode;

#[derive(Default)]
struct FixtureMapper;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DeltaPayload {
    text: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UsagePayload {
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
}

impl SseEventMapper for FixtureMapper {
    fn map_event(&mut self, event: &SseEvent) -> Result<Vec<UnifiedStreamEvent>, AiError> {
        match event.event_type() {
            "delta" => {
                #[derive(Deserialize)]
                struct VisibleDelta {
                    text: String,
                }
                let payload: VisibleDelta = parse_known_json(event)?;
                Ok(vec![UnifiedStreamEvent::TextDelta { text: payload.text }])
            }
            "strict_delta" => {
                let payload: DeltaPayload = parse_known_json(event)?;
                Ok(vec![UnifiedStreamEvent::TextDelta { text: payload.text }])
            }
            "usage" => {
                let payload: UsagePayload = parse_known_json(event)?;
                Ok(vec![UnifiedStreamEvent::Usage {
                    input_tokens: payload.input_tokens,
                    output_tokens: payload.output_tokens,
                }])
            }
            "complete" => {
                #[derive(Deserialize)]
                struct Completion {
                    status: String,
                }
                let payload: Completion = parse_known_json(event)?;
                if payload.status == "completed" {
                    Ok(vec![UnifiedStreamEvent::Completed])
                } else {
                    Err(AiError::malformed_event())
                }
            }
            "error" => Err(AiError::from_http(
                StatusCode::TOO_MANY_REQUESTS,
                event.data().as_bytes(),
            )),
            "message" if event.data() == "[DONE]" => Err(AiError::unexpected_eof()),
            _ => Ok(Vec::new()),
        }
    }
}

fn chunks_stream(
    chunks: Vec<Vec<u8>>,
) -> impl Stream<Item = Result<Vec<u8>, io::Error>> + Send + 'static {
    stream::iter(chunks.into_iter().map(Ok))
}

async fn collect_chunks(chunks: Vec<Vec<u8>>) -> Vec<Result<UnifiedStreamEvent, AiError>> {
    decode_sse(
        chunks_stream(chunks),
        FixtureMapper,
        CancellationToken::new(),
        Duration::from_secs(1),
    )
    .collect()
    .await
}

#[tokio::test]
async fn framing_supports_lf_crlf_multiline_comments_multiple_events_and_unknown_fields() {
    let bytes = concat!(
        ": heartbeat\r\n\r\n",
        "event: delta\r\n",
        "extension: ignored\r\n",
        "data: {\"text\":\r\n",
        "data: \"visible answer\",\"reasoning_content\":\"fixture-hidden-reasoning\",\"thought\":\"fixture-hidden-thought\",\"signature\":\"fixture-hidden-signature\"}\r\n\r\n",
        "event: extension\n",
        "data: {\"future\":true}\n\n",
        "event: usage\n",
        "data: {\"input_tokens\":7,\"output_tokens\":3}\n\n",
        "event: complete\n",
        "data: {\"status\":\"completed\"}\n\n"
    )
    .as_bytes()
    .to_vec();

    let events = collect_chunks(vec![bytes]).await;
    assert_eq!(
        events,
        vec![
            Ok(UnifiedStreamEvent::TextDelta {
                text: "visible answer".to_owned(),
            }),
            Ok(UnifiedStreamEvent::Usage {
                input_tokens: Some(7),
                output_tokens: Some(3),
            }),
            Ok(UnifiedStreamEvent::Completed),
        ]
    );
    let successful = events
        .iter()
        .map(|event| event.as_ref().unwrap())
        .collect::<Vec<_>>();
    let serialized = serde_json::to_string(&successful).unwrap();
    assert!(!serialized.contains("fixture-hidden-reasoning"));
    assert!(!serialized.contains("fixture-hidden-thought"));
    assert!(!serialized.contains("fixture-hidden-signature"));
}

#[tokio::test]
async fn utf8_code_point_can_cross_input_chunk_boundaries() {
    let bytes = "event: delta\ndata: {\"text\":\"跨块\"}\n\nevent: complete\ndata: {\"status\":\"completed\"}\n\n"
        .as_bytes();
    let split = bytes
        .windows(3)
        .position(|window| window == "跨".as_bytes())
        .unwrap()
        + 1;
    let events = collect_chunks(vec![bytes[..split].to_vec(), bytes[split..].to_vec()]).await;
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

#[tokio::test]
async fn malformed_known_json_oversized_data_vendor_error_and_done_only_are_failures() {
    let cases = [
        (
            b"event: strict_delta\ndata: {\"text\":7}\n\n".to_vec(),
            AppErrorCode::ProviderUnavailable,
        ),
        (
            format!(
                "event: delta\ndata: {}\n\n",
                "x".repeat(MAX_SSE_DATA_BYTES + 1)
            )
            .into_bytes(),
            AppErrorCode::ProviderUnavailable,
        ),
        (
            b"event: error\ndata: {\"error\":{\"type\":\"rate_limit_error\"}}\n\n".to_vec(),
            AppErrorCode::RateLimited,
        ),
        (
            b"data: [DONE]\n\n".to_vec(),
            AppErrorCode::ProviderUnavailable,
        ),
    ];

    for (bytes, code) in cases {
        let events = collect_chunks(vec![bytes]).await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].as_ref().unwrap_err().app_code(), code);
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, Ok(UnifiedStreamEvent::Completed)))
        );
    }
}

#[tokio::test]
async fn early_eof_after_delta_is_not_completion() {
    let events = collect_chunks(vec![
        b"event: delta\ndata: {\"text\":\"partial\"}\n\n".to_vec(),
    ])
    .await;
    assert_eq!(
        events.first(),
        Some(&Ok(UnifiedStreamEvent::TextDelta {
            text: "partial".to_owned(),
        }))
    );
    assert_eq!(
        events.last().unwrap().as_ref().unwrap_err().kind(),
        AiErrorKind::ProviderUnavailable
    );
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, Ok(UnifiedStreamEvent::Completed)))
    );
}

#[tokio::test]
async fn cancellation_between_chunks_drops_source_and_emits_no_later_event() {
    let (sender, receiver) = mpsc::channel::<Vec<u8>>(2);
    let source = stream::unfold(receiver, |mut receiver| async move {
        receiver
            .recv()
            .await
            .map(|bytes| (Ok::<_, io::Error>(bytes), receiver))
    });
    let cancel = CancellationToken::new();
    let mut events = decode_sse(
        source,
        FixtureMapper,
        cancel.clone(),
        Duration::from_secs(1),
    );

    sender
        .send(b"event: delta\ndata: {\"text\":\"first\"}\n\n".to_vec())
        .await
        .unwrap();
    assert_eq!(
        events.next().await.unwrap().unwrap(),
        UnifiedStreamEvent::TextDelta {
            text: "first".to_owned(),
        }
    );

    cancel.cancel();
    let error = events.next().await.unwrap().unwrap_err();
    assert_eq!(error.kind(), AiErrorKind::Cancelled);
    assert_eq!(error.app_code(), AppErrorCode::ImportCancelled);
    assert!(
        sender
            .send(b"event: complete\ndata: {\"status\":\"completed\"}\n\n".to_vec())
            .await
            .is_err()
    );
    assert!(events.next().await.is_none());
}

struct PendingDropStream {
    dropped: Arc<AtomicBool>,
}

impl Stream for PendingDropStream {
    type Item = Result<Vec<u8>, io::Error>;

    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        Poll::Pending
    }
}

impl Drop for PendingDropStream {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
    }
}

#[tokio::test]
async fn cancellation_prompt_drops_pending_response_and_idle_timeout_is_bounded() {
    let dropped = Arc::new(AtomicBool::new(false));
    let cancel = CancellationToken::new();
    let mut events = decode_sse(
        PendingDropStream {
            dropped: dropped.clone(),
        },
        FixtureMapper,
        cancel.clone(),
        Duration::from_secs(1),
    );
    cancel.cancel();
    assert_eq!(
        events.next().await.unwrap().unwrap_err().kind(),
        AiErrorKind::Cancelled
    );
    assert!(dropped.load(Ordering::SeqCst));

    let mut idle = decode_sse(
        stream::pending::<Result<Vec<u8>, io::Error>>(),
        FixtureMapper,
        CancellationToken::new(),
        Duration::from_millis(20),
    );
    assert_eq!(
        idle.next().await.unwrap().unwrap_err().kind(),
        AiErrorKind::ProviderUnavailable
    );
    assert!(idle.next().await.is_none());
}

#[test]
fn raw_sse_event_debug_never_exposes_data() {
    let event = SseEvent::new_for_test(
        "delta",
        r#"{"text":"fixture-visible-body","thought":"fixture-hidden-body"}"#,
    );
    let debug = format!("{event:?}");
    assert!(!debug.contains("fixture-visible-body"));
    assert!(!debug.contains("fixture-hidden-body"));
}
