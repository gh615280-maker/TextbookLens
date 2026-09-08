use std::{fmt, time::Duration};

#[cfg(test)]
use std::net::IpAddr;

use futures_util::StreamExt;
use reqwest::{
    Client, Method, StatusCode, Url,
    header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue},
    redirect,
};
use secrecy::{ExposeSecret, SecretString};
use serde::{Serialize, de::DeserializeOwned};
use tokio_util::sync::CancellationToken;
#[cfg(test)]
use zeroize::Zeroize;
use zeroize::Zeroizing;

use super::{
    error::AiError,
    provider::{ProviderStream, validate_credential},
    stream::{MAX_SSE_DATA_BYTES, SseEventMapper, decode_sse},
};
use crate::domain::{KimiApiRegion, ProviderKind};

pub const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
const USER_AGENT: &str = concat!("TextbookLens/", env!("CARGO_PKG_VERSION"));

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TransportPolicy {
    pub connect_timeout: Duration,
    pub request_idle_timeout: Duration,
    pub max_response_bytes: usize,
    pub max_sse_data_bytes: usize,
}

impl Default for TransportPolicy {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(30),
            request_idle_timeout: Duration::from_secs(120),
            max_response_bytes: MAX_RESPONSE_BYTES,
            max_sse_data_bytes: MAX_SSE_DATA_BYTES,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CredentialHeader {
    Bearer,
    XApiKey,
    XGoogApiKey,
}

pub struct ProviderHttpRequest {
    method: Method,
    endpoint: String,
    headers: HeaderMap,
    credential_header: CredentialHeader,
    body: Option<Zeroizing<Vec<u8>>>,
}

impl ProviderHttpRequest {
    pub fn empty(
        method: Method,
        endpoint: impl Into<String>,
        credential_header: CredentialHeader,
    ) -> Result<Self, AiError> {
        let endpoint = endpoint.into();
        validate_endpoint_text(&endpoint)?;
        Ok(Self {
            method,
            endpoint,
            headers: HeaderMap::new(),
            credential_header,
            body: None,
        })
    }

    pub fn json<T: Serialize + ?Sized>(
        method: Method,
        endpoint: impl Into<String>,
        credential_header: CredentialHeader,
        value: &T,
    ) -> Result<Self, AiError> {
        let mut request = Self::empty(method, endpoint, credential_header)?;
        let body = serde_json::to_vec(value).map_err(|_| AiError::invalid_input())?;
        request.body = Some(Zeroizing::new(body));
        request
            .headers
            .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        Ok(request)
    }

    pub fn with_header(mut self, name: HeaderName, value: HeaderValue) -> Result<Self, AiError> {
        if is_credential_header(&name) {
            return Err(AiError::invalid_input());
        }
        self.headers.insert(name, value);
        Ok(self)
    }

    #[cfg(test)]
    pub(crate) fn zeroize_body_for_test(&mut self) -> bool {
        if let Some(body) = self.body.as_mut() {
            body.zeroize();
            body.is_empty()
        } else {
            true
        }
    }
}

impl fmt::Debug for ProviderHttpRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderHttpRequest")
            .field("method", &self.method)
            .field("endpoint", &self.endpoint)
            .field("header_count", &self.headers.len())
            .field("credential_header", &self.credential_header)
            .field("body", &self.body.as_ref().map(|_| "[REDACTED]"))
            .finish()
    }
}

pub struct ProviderTransport {
    kind: ProviderKind,
    client: Client,
    origin: Url,
    policy: TransportPolicy,
}

impl ProviderTransport {
    pub fn new(kind: ProviderKind) -> Result<Self, AiError> {
        let origin = Url::parse(production_origin(&kind).ok_or_else(AiError::invalid_input)?)
            .map_err(|_| AiError::invalid_input())?;
        if origin.scheme() != "https" {
            return Err(AiError::invalid_input());
        }
        Self::build(kind, origin, TransportPolicy::default())
    }

    pub(crate) fn new_for_kimi_region(region: KimiApiRegion) -> Result<Self, AiError> {
        let origin = Url::parse(crate::ai::kimi_region::production_origin(region))
            .map_err(|_| AiError::invalid_input())?;
        Self::build(ProviderKind::Kimi, origin, TransportPolicy::default())
    }

    pub const fn policy(&self) -> TransportPolicy {
        self.policy
    }

    pub async fn send_bounded(
        &self,
        request: ProviderHttpRequest,
        credential: &SecretString,
        cancel: CancellationToken,
    ) -> Result<BoundedResponse, AiError> {
        let response = self.send(request, credential, &cancel).await?;
        let status = response.status();
        let body = read_bounded_body(response, &cancel, self.policy).await?;
        if !status.is_success() {
            return Err(AiError::from_provider_http(
                &self.kind,
                status,
                body.as_slice(),
            ));
        }
        Ok(BoundedResponse { status, body })
    }

    pub async fn send_bounded_delete_idempotent(
        &self,
        request: ProviderHttpRequest,
        credential: &SecretString,
        cancel: CancellationToken,
    ) -> Result<(), AiError> {
        let response = self.send(request, credential, &cancel).await?;
        let status = response.status();
        let body = read_bounded_body(response, &cancel, self.policy).await?;
        if status.is_success() || status == StatusCode::NOT_FOUND {
            return Ok(());
        }
        Err(AiError::from_provider_http(&self.kind, status, &body))
    }

    pub async fn send_stream(
        &self,
        request: ProviderHttpRequest,
        credential: &SecretString,
        cancel: CancellationToken,
    ) -> Result<ProviderResponseStream, AiError> {
        let response = self.send(request, credential, &cancel).await?;
        let status = response.status();
        if !status.is_success() {
            let body = read_bounded_body(response, &cancel, self.policy).await?;
            return Err(AiError::from_provider_http(
                &self.kind,
                status,
                body.as_slice(),
            ));
        }
        Ok(ProviderResponseStream {
            response: Some(response),
            cancel,
            idle_timeout: self.policy.request_idle_timeout,
        })
    }

    async fn send(
        &self,
        request: ProviderHttpRequest,
        credential: &SecretString,
        cancel: &CancellationToken,
    ) -> Result<reqwest::Response, AiError> {
        validate_credential(credential)?;
        if cancel.is_cancelled() {
            return Err(AiError::cancelled());
        }
        let url = self.resolve_endpoint(&request.endpoint)?;
        let ProviderHttpRequest {
            method,
            headers,
            credential_header,
            body,
            ..
        } = request;

        let mut builder = self.client.request(method, url).headers(headers);
        if let Some(body) = body.as_ref() {
            builder = builder.body(body.as_slice().to_vec());
        }
        let (name, value) = credential_header_value(credential_header, credential)?;
        builder = builder.header(name, value);
        let request = builder.build().map_err(|_| AiError::invalid_input())?;
        drop(body);

        let send = self.client.execute(request);
        tokio::pin!(send);
        tokio::select! {
            biased;
            _ = cancel.cancelled() => Err(AiError::cancelled()),
            result = tokio::time::timeout(self.policy.request_idle_timeout, &mut send) => {
                match result {
                    Ok(Ok(response)) => Ok(response),
                    Ok(Err(error)) => Err(AiError::from_reqwest(&error)),
                    Err(_) => Err(AiError::provider_unavailable()),
                }
            }
        }
    }

    fn build(
        kind: ProviderKind,
        mut origin: Url,
        policy: TransportPolicy,
    ) -> Result<Self, AiError> {
        if origin.username() != "" || origin.password().is_some() || origin.query().is_some() {
            return Err(AiError::invalid_input());
        }
        origin.set_fragment(None);
        if !origin.path().ends_with('/') {
            let path = format!("{}/", origin.path());
            origin.set_path(&path);
        }

        let client = Client::builder()
            .redirect(redirect::Policy::none())
            .referer(false)
            .connect_timeout(policy.connect_timeout)
            .gzip(true)
            .brotli(true)
            .user_agent(USER_AGENT)
            .no_proxy()
            .retry(reqwest::retry::never())
            .tls_backend_rustls()
            .build()
            .map_err(|_| AiError::provider_unavailable())?;

        Ok(Self {
            kind,
            client,
            origin,
            policy,
        })
    }

    fn resolve_endpoint(&self, endpoint: &str) -> Result<Url, AiError> {
        validate_endpoint_text(endpoint)?;
        let endpoint = endpoint.strip_prefix('/').unwrap_or(endpoint);
        let url = self
            .origin
            .join(endpoint)
            .map_err(|_| AiError::invalid_input())?;
        if url.scheme() != self.origin.scheme()
            || url.host_str() != self.origin.host_str()
            || url.port_or_known_default() != self.origin.port_or_known_default()
            || !url.path().starts_with(self.origin.path())
            || url.fragment().is_some()
            || url
                .query_pairs()
                .any(|(name, _)| is_sensitive_query_name(&name))
        {
            return Err(AiError::invalid_input());
        }
        Ok(url)
    }

    #[cfg(test)]
    pub(crate) fn new_for_test(kind: ProviderKind, origin: &str) -> Result<Self, AiError> {
        Self::new_for_test_with_timeout(kind, origin, Duration::from_secs(120))
    }

    #[cfg(test)]
    pub(crate) fn new_for_test_with_timeout(
        kind: ProviderKind,
        origin: &str,
        request_idle_timeout: Duration,
    ) -> Result<Self, AiError> {
        let origin = Url::parse(origin).map_err(|_| AiError::invalid_input())?;
        if !is_loopback_origin(&origin) {
            return Err(AiError::invalid_input());
        }
        let policy = TransportPolicy {
            request_idle_timeout,
            ..TransportPolicy::default()
        };
        Self::build(kind, origin, policy)
    }

    #[cfg(test)]
    pub(crate) fn origin_for_test(&self) -> &str {
        self.origin.as_str()
    }

    #[cfg(test)]
    pub(crate) fn resolve_endpoint_for_test(&self, endpoint: &str) -> Result<Url, AiError> {
        self.resolve_endpoint(endpoint)
    }
}

impl fmt::Debug for ProviderTransport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderTransport")
            .field("kind", &self.kind)
            .field("origin", &self.origin)
            .field("policy", &self.policy)
            .finish_non_exhaustive()
    }
}

pub struct BoundedResponse {
    status: StatusCode,
    body: Zeroizing<Vec<u8>>,
}

impl BoundedResponse {
    pub const fn status(&self) -> StatusCode {
        self.status
    }

    pub fn json<T: DeserializeOwned>(&self) -> Result<T, AiError> {
        serde_json::from_slice(self.body.as_slice()).map_err(|_| AiError::malformed_event())
    }
}

impl fmt::Debug for BoundedResponse {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BoundedResponse")
            .field("status", &self.status)
            .field("body", &"[REDACTED]")
            .finish()
    }
}

pub struct ProviderResponseStream {
    response: Option<reqwest::Response>,
    cancel: CancellationToken,
    idle_timeout: Duration,
}

impl ProviderResponseStream {
    pub fn decode<M: SseEventMapper>(mut self, mapper: M) -> ProviderStream {
        let response = self.response.take().expect("provider response stream");
        decode_sse(
            response.bytes_stream(),
            mapper,
            self.cancel.clone(),
            self.idle_timeout,
        )
    }
}

impl fmt::Debug for ProviderResponseStream {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderResponseStream")
            .field("response", &"[REDACTED]")
            .field("cancelled", &self.cancel.is_cancelled())
            .field("idle_timeout", &self.idle_timeout)
            .finish()
    }
}

async fn read_bounded_body(
    response: reqwest::Response,
    cancel: &CancellationToken,
    policy: TransportPolicy,
) -> Result<Zeroizing<Vec<u8>>, AiError> {
    let mut stream = response.bytes_stream();
    let mut body = Zeroizing::new(Vec::new());
    loop {
        let next = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(AiError::cancelled()),
            result = tokio::time::timeout(policy.request_idle_timeout, stream.next()) => {
                result.map_err(|_| AiError::provider_unavailable())?
            }
        };
        let Some(chunk) = next else {
            return Ok(body);
        };
        let chunk = chunk.map_err(|error| AiError::from_reqwest(&error))?;
        if chunk.len() > policy.max_response_bytes.saturating_sub(body.len()) {
            return Err(AiError::provider_unavailable());
        }
        body.extend_from_slice(&chunk);
    }
}

fn production_origin(kind: &ProviderKind) -> Option<&'static str> {
    match kind {
        ProviderKind::OpenAi => Some("https://api.openai.com/"),
        ProviderKind::Gemini => Some("https://generativelanguage.googleapis.com/"),
        ProviderKind::Anthropic => Some("https://api.anthropic.com/"),
        ProviderKind::DeepSeek => Some("https://api.deepseek.com/"),
        ProviderKind::Kimi => None,
        ProviderKind::Ollama | ProviderKind::LmStudio => None,
    }
}

fn credential_header_value(
    placement: CredentialHeader,
    credential: &SecretString,
) -> Result<(HeaderName, HeaderValue), AiError> {
    let (name, owned) = credential_buffer(placement, credential);
    let mut value =
        HeaderValue::from_bytes(owned.as_slice()).map_err(|_| AiError::invalid_input())?;
    value.set_sensitive(true);
    Ok((name, value))
}

fn credential_buffer(
    placement: CredentialHeader,
    credential: &SecretString,
) -> (HeaderName, Zeroizing<Vec<u8>>) {
    let secret = credential.expose_secret();
    let mut owned = Zeroizing::new(Vec::with_capacity(secret.len() + 7));
    let name = match placement {
        CredentialHeader::Bearer => {
            owned.extend_from_slice(b"Bearer ");
            AUTHORIZATION
        }
        CredentialHeader::XApiKey => HeaderName::from_static("x-api-key"),
        CredentialHeader::XGoogApiKey => HeaderName::from_static("x-goog-api-key"),
    };
    owned.extend_from_slice(secret.as_bytes());
    (name, owned)
}

#[cfg(test)]
pub(crate) fn zeroize_credential_buffer_for_test(credential: &SecretString) -> bool {
    let (_, mut buffer) = credential_buffer(CredentialHeader::Bearer, credential);
    buffer.zeroize();
    buffer.is_empty()
}

fn validate_endpoint_text(endpoint: &str) -> Result<(), AiError> {
    let lower = endpoint.to_ascii_lowercase();
    if endpoint.is_empty()
        || endpoint.trim() != endpoint
        || endpoint.starts_with("//")
        || endpoint.contains('\\')
        || lower.contains("://")
        || endpoint.split(['/', '?']).any(|part| {
            let part = part.to_ascii_lowercase();
            part == "." || part == ".." || part.contains("%2e")
        })
        || endpoint.chars().any(char::is_control)
    {
        return Err(AiError::invalid_input());
    }
    Ok(())
}

#[cfg(test)]
fn is_loopback_origin(origin: &Url) -> bool {
    if !matches!(origin.scheme(), "http" | "https")
        || origin.username() != ""
        || origin.password().is_some()
        || origin.query().is_some()
        || origin.fragment().is_some()
    {
        return false;
    }
    match origin.host_str() {
        Some("localhost") => true,
        Some(host) => host
            .trim_start_matches('[')
            .trim_end_matches(']')
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback()),
        None => false,
    }
}

fn is_credential_header(name: &HeaderName) -> bool {
    matches!(
        name.as_str(),
        "authorization" | "proxy-authorization" | "x-api-key" | "x-goog-api-key"
    )
}

fn is_sensitive_query_name(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "key" | "api_key" | "apikey" | "token" | "access_token"
    )
}
