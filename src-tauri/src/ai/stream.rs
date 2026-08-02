use std::{collections::VecDeque, fmt, pin::Pin, time::Duration};

use eventsource_stream::{EventStream, Eventsource};
use futures_util::{Stream, StreamExt, stream};
use serde::de::DeserializeOwned;
use tokio_util::sync::CancellationToken;
use zeroize::{Zeroize, Zeroizing};

use super::{
    error::AiError,
    provider::{ProviderStream, UnifiedStreamEvent},
};

pub const MAX_SSE_DATA_BYTES: usize = 1024 * 1024;

pub struct SseEvent {
    event_type: String,
    data: Zeroizing<String>,
}

impl SseEvent {
    pub fn event_type(&self) -> &str {
        &self.event_type
    }

    pub fn data(&self) -> &str {
        self.data.as_str()
    }

    #[cfg(test)]
    pub(crate) fn new_for_test(event_type: impl Into<String>, data: impl Into<String>) -> Self {
        Self {
            event_type: event_type.into(),
            data: Zeroizing::new(data.into()),
        }
    }
}

impl fmt::Debug for SseEvent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SseEvent")
            .field("event_type", &self.event_type)
            .field("data", &"[REDACTED]")
            .finish()
    }
}

pub trait SseEventMapper: Send + 'static {
    /// A mapper may emit `Completed` only after parsing the provider's explicit
    /// successful terminal signal. EOF and generic `[DONE]` markers are not enough.
    fn map_event(&mut self, event: &SseEvent) -> Result<Vec<UnifiedStreamEvent>, AiError>;
}

pub fn parse_known_json<T: DeserializeOwned>(event: &SseEvent) -> Result<T, AiError> {
    serde_json::from_str(event.data()).map_err(|_| AiError::malformed_event())
}

pub fn decode_sse<S, B, E, M>(
    source: S,
    mapper: M,
    cancel: CancellationToken,
    idle_timeout: Duration,
) -> ProviderStream
where
    S: Stream<Item = Result<B, E>> + Send + 'static,
    B: AsRef<[u8]> + Send + 'static,
    E: Send + 'static,
    M: SseEventMapper,
{
    let state = DecodeState {
        source: Some(Box::pin(
            BoundedSseSource::new(source, MAX_SSE_DATA_BYTES).eventsource(),
        )),
        mapper,
        cancel,
        idle_timeout,
        pending: VecDeque::new(),
        completed: false,
        terminal: false,
    };

    Box::pin(stream::unfold(state, |mut state| async move {
        loop {
            if let Some(item) = state.pending.pop_front() {
                return Some((item, state));
            }
            if state.terminal || state.completed {
                state.source.take();
                return None;
            }

            let cancel = state.cancel.clone();
            let idle_timeout = state.idle_timeout;
            let outcome = {
                let mut source = state.source.as_mut().expect("active SSE source").as_mut();
                tokio::select! {
                    biased;
                    _ = cancel.cancelled() => NextEvent::Cancelled,
                    result = tokio::time::timeout(idle_timeout, source.next()) => {
                        match result {
                            Ok(item) => NextEvent::Ready(item),
                            Err(_) => NextEvent::TimedOut,
                        }
                    }
                }
            };

            let parsed = match outcome {
                NextEvent::Cancelled => {
                    state.source.take();
                    state.terminal = true;
                    return Some((Err(AiError::cancelled()), state));
                }
                NextEvent::TimedOut => {
                    state.source.take();
                    state.terminal = true;
                    return Some((Err(AiError::provider_unavailable()), state));
                }
                NextEvent::Ready(None) => {
                    state.source.take();
                    state.terminal = true;
                    return Some((Err(AiError::unexpected_eof()), state));
                }
                NextEvent::Ready(Some(Err(_))) => {
                    state.source.take();
                    state.terminal = true;
                    return Some((Err(AiError::malformed_event()), state));
                }
                NextEvent::Ready(Some(Ok(event))) => event,
            };

            if parsed.data.len() > MAX_SSE_DATA_BYTES {
                state.source.take();
                state.terminal = true;
                return Some((Err(AiError::provider_unavailable()), state));
            }

            let event = SseEvent {
                event_type: parsed.event,
                data: Zeroizing::new(parsed.data),
            };
            let mapped = match state.mapper.map_event(&event) {
                Ok(mapped) => mapped,
                Err(error) => {
                    state.source.take();
                    state.terminal = true;
                    return Some((Err(error), state));
                }
            };

            let completion_count = mapped
                .iter()
                .filter(|event| matches!(event, UnifiedStreamEvent::Completed))
                .count();
            let completed = completion_count == 1
                && matches!(mapped.last(), Some(UnifiedStreamEvent::Completed));
            if completion_count > 1 || (completion_count == 1 && !completed) {
                state.source.take();
                state.terminal = true;
                return Some((Err(AiError::malformed_event()), state));
            }

            if completed {
                state.completed = true;
                state.source.take();
            }
            state.pending.extend(mapped.into_iter().map(Ok));
        }
    }))
}

struct DecodeState<S, M> {
    source: Option<Pin<Box<EventStream<S>>>>,
    mapper: M,
    cancel: CancellationToken,
    idle_timeout: Duration,
    pending: VecDeque<Result<UnifiedStreamEvent, AiError>>,
    completed: bool,
    terminal: bool,
}

enum NextEvent<T> {
    Cancelled,
    TimedOut,
    Ready(T),
}

/// Enforces the resource bound before `eventsource-stream` accumulates a full
/// event. Framing and UTF-8 decoding remain exclusively owned by that crate.
struct BoundedSseSource<S> {
    source: Pin<Box<S>>,
    line: Zeroizing<Vec<u8>>,
    event_data_bytes: usize,
    saw_data_field: bool,
    skip_lf_after_cr: bool,
    max_data_bytes: usize,
}

impl<S> BoundedSseSource<S> {
    fn new(source: S, max_data_bytes: usize) -> Self {
        Self {
            source: Box::pin(source),
            line: Zeroizing::new(Vec::new()),
            event_data_bytes: 0,
            saw_data_field: false,
            skip_lf_after_cr: false,
            max_data_bytes,
        }
    }

    fn scan_chunk(&mut self, chunk: &[u8]) -> Result<(), ()> {
        for &byte in chunk {
            if self.skip_lf_after_cr {
                self.skip_lf_after_cr = false;
                if byte == b'\n' {
                    continue;
                }
            }
            match byte {
                b'\r' => {
                    self.finish_line()?;
                    self.skip_lf_after_cr = true;
                }
                b'\n' => self.finish_line()?,
                _ => {
                    self.line.push(byte);
                    if self.line.len() > self.max_data_bytes.saturating_add(6) {
                        return Err(());
                    }
                }
            }
        }
        Ok(())
    }

    fn finish_line(&mut self) -> Result<(), ()> {
        if self.line.is_empty() {
            self.event_data_bytes = 0;
            self.saw_data_field = false;
            return Ok(());
        }

        let value_bytes = if self.line.as_slice() == b"data" {
            Some(0)
        } else {
            self.line
                .as_slice()
                .strip_prefix(b"data:")
                .map(|value| value.strip_prefix(b" ").unwrap_or(value).len())
        };
        if let Some(value_bytes) = value_bytes {
            let separator = usize::from(self.saw_data_field);
            self.event_data_bytes = self
                .event_data_bytes
                .checked_add(separator)
                .and_then(|length| length.checked_add(value_bytes))
                .ok_or(())?;
            if self.event_data_bytes > self.max_data_bytes {
                return Err(());
            }
            self.saw_data_field = true;
        }
        self.line.zeroize();
        Ok(())
    }
}

impl<S, B, E> Stream for BoundedSseSource<S>
where
    S: Stream<Item = Result<B, E>>,
    B: AsRef<[u8]>,
{
    type Item = Result<B, BoundedSourceError<E>>;

    fn poll_next(
        mut self: Pin<&mut Self>,
        context: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        let this = self.as_mut().get_mut();
        match this.source.as_mut().poll_next(context) {
            std::task::Poll::Ready(Some(Ok(chunk))) => {
                if this.scan_chunk(chunk.as_ref()).is_err() {
                    std::task::Poll::Ready(Some(Err(BoundedSourceError::DataTooLarge)))
                } else {
                    std::task::Poll::Ready(Some(Ok(chunk)))
                }
            }
            std::task::Poll::Ready(Some(Err(error))) => {
                std::task::Poll::Ready(Some(Err(BoundedSourceError::Upstream(error))))
            }
            other => {
                other.map(|item| item.map(|result| result.map_err(BoundedSourceError::Upstream)))
            }
        }
    }
}

enum BoundedSourceError<E> {
    Upstream(E),
    DataTooLarge,
}
