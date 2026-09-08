use std::{collections::VecDeque, time::Duration};

use futures_util::{StreamExt, stream};
use reqwest::{Client, Method, Response};
use serde_json::Value;
use tokio_util::sync::CancellationToken;
use zeroize::Zeroizing;

use crate::{
    ai::{
        error::AiError,
        provider::ProviderStream,
        stream::{SseEvent, SseEventMapper, decode_sse},
    },
    domain::UnifiedStreamEvent,
};

pub(super) const INFERENCE_TIMEOUT: Duration = Duration::from_secs(300);
const MAX_BODY: usize = 4 * 1024 * 1024;

/// Only numeric IPv4 loopback addresses are constructible. Never uses proxies,
/// redirects, remote credentials, retries, or a caller-provided origin.
#[derive(Clone)]
pub(super) struct LocalHttp {
    client: Client,
    port: u16,
}

impl LocalHttp {
    pub fn new(port: u16) -> Result<Self, AiError> {
        if port == 0 {
            return Err(AiError::invalid_input());
        }
        Ok(Self {
            client: Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .retry(reqwest::retry::never())
                .connect_timeout(Duration::from_secs(1))
                .build()
                .map_err(|_| AiError::provider_unavailable())?,
            port,
        })
    }

    pub async fn request(
        &self,
        path: &'static str,
        body: Option<Value>,
        cancel: &CancellationToken,
        timeout: Duration,
    ) -> Result<Response, AiError> {
        let mut request = self.client.request(
            if body.is_some() {
                Method::POST
            } else {
                Method::GET
            },
            format!("http://127.0.0.1:{}{path}", self.port),
        );
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(AiError::cancelled()),
            response = tokio::time::timeout(timeout, request.send()) => response.map_err(|_| AiError::provider_unavailable())?.map_err(|e| AiError::from_reqwest(&e))?,
        };
        match response.status().as_u16() {
            200..=299 => Ok(response),
            401 | 403 => Err(AiError::invalid_api_key()),
            404 => Err(AiError::model_not_found()),
            _ => Err(AiError::provider_unavailable()),
        }
    }

    pub async fn json(
        &self,
        path: &'static str,
        body: Option<Value>,
        cancel: &CancellationToken,
        timeout: Duration,
    ) -> Result<Value, AiError> {
        let response = self.request(path, body, cancel, timeout).await?;
        let read = async {
            let mut bytes = Zeroizing::new(Vec::new());
            let mut source = response.bytes_stream();
            while let Some(chunk) = source.next().await {
                let chunk = chunk.map_err(|_| AiError::provider_unavailable())?;
                if bytes.len().saturating_add(chunk.len()) > MAX_BODY {
                    return Err(AiError::provider_unavailable());
                }
                bytes.extend_from_slice(&chunk);
            }
            serde_json::from_slice(&bytes).map_err(|_| AiError::provider_unavailable())
        };
        tokio::select! {
            biased;
            _ = cancel.cancelled() => Err(AiError::cancelled()),
            result = tokio::time::timeout(timeout, read) => result.map_err(|_| AiError::provider_unavailable())?,
        }
    }
}

pub(super) fn lmstudio_stream(response: Response, cancel: CancellationToken) -> ProviderStream {
    decode_sse(
        response.bytes_stream(),
        ChatMapper,
        cancel,
        INFERENCE_TIMEOUT,
    )
}

struct ChatMapper;
impl SseEventMapper for ChatMapper {
    fn map_event(&mut self, event: &SseEvent) -> Result<Vec<UnifiedStreamEvent>, AiError> {
        if event.data() == "[DONE]" {
            return Ok(Vec::new());
        }
        let value: Value =
            serde_json::from_str(event.data()).map_err(|_| AiError::malformed_event())?;
        if value.get("error").is_some() {
            return Err(AiError::provider_unavailable());
        }
        let choices = value["choices"]
            .as_array()
            .ok_or_else(AiError::malformed_event)?;
        let mut events = Vec::new();
        for choice in choices {
            if choice["index"].as_u64().unwrap_or(0) != 0 {
                continue;
            }
            if let Some(text) = choice["delta"]["content"]
                .as_str()
                .filter(|v| !v.is_empty())
            {
                events.push(UnifiedStreamEvent::TextDelta {
                    text: text.to_owned(),
                });
            }
            match choice["finish_reason"].as_str() {
                Some("stop") => events.push(UnifiedStreamEvent::Completed),
                Some("length") => return Err(AiError::context_too_large()),
                Some(_) => return Err(AiError::refused()),
                None => {}
            }
        }
        Ok(events)
    }
}

pub(super) fn ollama_stream(response: Response, cancel: CancellationToken) -> ProviderStream {
    let source = Box::pin(response.bytes_stream());
    let initial = (
        source,
        Zeroizing::new(Vec::<u8>::new()),
        VecDeque::new(),
        false,
        cancel,
    );
    Box::pin(stream::unfold(
        initial,
        |(mut source, mut buffer, mut pending, mut terminal, cancel)| async move {
            loop {
                if cancel.is_cancelled() && (!terminal || !pending.is_empty()) {
                    pending.clear();
                    return Some((
                        Err(AiError::cancelled()),
                        (source, buffer, pending, true, cancel),
                    ));
                }
                if let Some(event) = pending.pop_front() {
                    return Some((Ok(event), (source, buffer, pending, terminal, cancel)));
                }
                if terminal {
                    return None;
                }
                let outcome: Result<(), AiError> = async {
                if cancel.is_cancelled() { return Err(AiError::cancelled()); }
                if let Some(end) = buffer.iter().position(|byte| *byte == b'\n') {
                    let line: Vec<u8> = buffer.drain(..=end).collect();
                    if line.iter().all(u8::is_ascii_whitespace) { return Ok(()); }
                    let value: Value = serde_json::from_slice(&line).map_err(|_| AiError::malformed_event())?;
                    if value.get("error").is_some() { return Err(AiError::provider_unavailable()); }
                    if let Some(text) = value["message"]["content"].as_str().filter(|v| !v.is_empty()) {
                        pending.push_back(UnifiedStreamEvent::TextDelta { text: text.to_owned() });
                    }
                    if value["done"] == true {
                        if value["done_reason"] == "length" { return Err(AiError::context_too_large()); }
                        pending.push_back(UnifiedStreamEvent::Completed);
                        terminal = true;
                    }
                    return Ok(());
                }
                let chunk = tokio::select! {
                    biased;
                    _ = cancel.cancelled() => return Err(AiError::cancelled()),
                    chunk = tokio::time::timeout(INFERENCE_TIMEOUT, source.next()) => chunk.map_err(|_| AiError::provider_unavailable())?,
                };
                match chunk {
                    Some(Ok(chunk)) => {
                        if buffer.len().saturating_add(chunk.len()) > MAX_BODY { return Err(AiError::malformed_event()); }
                        buffer.extend_from_slice(&chunk);
                    },
                    Some(Err(_)) => return Err(AiError::provider_unavailable()),
                    None if !buffer.is_empty() => buffer.push(b'\n'),
                    None => return Err(AiError::unexpected_eof()),
                }
                Ok(())
            }.await;
                if let Err(error) = outcome {
                    pending.clear();
                    return Some((Err(error), (source, buffer, pending, true, cancel)));
                }
            }
        },
    ))
}
