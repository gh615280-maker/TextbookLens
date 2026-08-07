use std::{fmt, time::Duration};

use futures_util::StreamExt;
use reqwest::{Client, StatusCode, Url, multipart};
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use tokio_util::sync::CancellationToken;
use zeroize::Zeroizing;

use crate::{
    ai::{error::AiError, provider::validate_credential},
    errors::{AppError, AppErrorCode, AppResult},
};

const PRODUCTION_ORIGIN: &str = "https://api.moonshot.ai/v1/";
pub const MAX_FILE_BYTES: usize = 100 * 1024 * 1024;
pub const MAX_EXTRACTED_CONTENT_BYTES: usize = 64 * 1024 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileState {
    Processing,
    Ready,
    Failed,
}

pub struct UploadedFile {
    id: SecretString,
}

impl UploadedFile {
    pub fn id(&self) -> &SecretString {
        &self.id
    }
}

impl fmt::Debug for UploadedFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UploadedFile")
            .field("id", &"[REDACTED]")
            .finish()
    }
}

pub struct KimiFilesClient {
    client: Client,
    origin: Url,
    max_extracted_content_bytes: usize,
}

impl KimiFilesClient {
    pub fn new() -> AppResult<Self> {
        Self::build(PRODUCTION_ORIGIN, false, MAX_EXTRACTED_CONTENT_BYTES)
    }

    #[cfg(test)]
    pub(crate) fn new_for_test(origin: &str) -> AppResult<Self> {
        Self::build(origin, true, MAX_EXTRACTED_CONTENT_BYTES)
    }

    #[cfg(test)]
    pub(crate) fn new_for_test_with_content_limit(origin: &str, limit: usize) -> AppResult<Self> {
        if limit == 0 || limit > MAX_EXTRACTED_CONTENT_BYTES {
            return Err(invalid());
        }
        Self::build(origin, true, limit)
    }

    fn build(
        origin: &str,
        allow_loopback: bool,
        max_extracted_content_bytes: usize,
    ) -> AppResult<Self> {
        let mut origin = Url::parse(origin).map_err(|_| invalid())?;
        let is_loopback = origin.host_str().is_some_and(|host| {
            host == "localhost"
                || host
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|ip| ip.is_loopback())
        });
        if origin.username() != ""
            || origin.password().is_some()
            || origin.query().is_some()
            || (!allow_loopback && origin.scheme() != "https")
            || (allow_loopback && !is_loopback)
        {
            return Err(invalid());
        }
        origin.set_fragment(None);
        if !origin.path().ends_with('/') {
            origin.set_path(&format!("{}/", origin.path()));
        }
        let client = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .referer(false)
            .connect_timeout(Duration::from_secs(30))
            .timeout(REQUEST_TIMEOUT)
            .no_proxy()
            .retry(reqwest::retry::never())
            .tls_backend_rustls()
            .build()
            .map_err(|_| unavailable())?;
        Ok(Self {
            client,
            origin,
            max_extracted_content_bytes,
        })
    }

    pub async fn upload(
        &self,
        credential: &SecretString,
        bytes: Vec<u8>,
        cancel: CancellationToken,
    ) -> AppResult<UploadedFile> {
        validate_credential(credential).map_err(AiError::into_app_error)?;
        if bytes.is_empty() || bytes.len() > MAX_FILE_BYTES {
            return Err(invalid());
        }
        let bytes = Zeroizing::new(bytes);
        let part = multipart::Part::bytes(bytes.as_slice().to_vec())
            .file_name("textbook.pdf")
            .mime_str("application/pdf")
            .map_err(|_| invalid())?;
        let form = multipart::Form::new()
            .text("purpose", "file-extract")
            .part("file", part);
        let request = self
            .client
            .post(self.url("files")?)
            .bearer_auth(credential.expose_secret())
            .multipart(form);
        let response = execute(request, cancel.clone()).await?;
        drop(bytes);
        let body = bounded(response, 1024 * 1024, cancel).await?;
        let value: FileEnvelope = serde_json::from_slice(&body).map_err(|_| unavailable())?;
        validate_file_id(&value.id)?;
        Ok(UploadedFile {
            id: SecretString::from(value.id),
        })
    }

    pub async fn status(
        &self,
        credential: &SecretString,
        file_id: &SecretString,
        cancel: CancellationToken,
    ) -> AppResult<FileState> {
        let response = execute(
            self.client
                .get(self.file_url(file_id, "")?)
                .bearer_auth(credential.expose_secret()),
            cancel.clone(),
        )
        .await?;
        let body = bounded(response, 1024 * 1024, cancel).await?;
        let value: FileEnvelope = serde_json::from_slice(&body).map_err(|_| unavailable())?;
        match value.status.as_deref() {
            Some("ok" | "ready" | "processed" | "completed") => Ok(FileState::Ready),
            Some("error" | "failed") => Ok(FileState::Failed),
            Some("pending" | "processing" | "uploaded" | "running") => Ok(FileState::Processing),
            _ => Err(unavailable()),
        }
    }

    pub async fn content(
        &self,
        credential: &SecretString,
        file_id: &SecretString,
        cancel: CancellationToken,
    ) -> AppResult<String> {
        let response = execute(
            self.client
                .get(self.file_url(file_id, "content")?)
                .bearer_auth(credential.expose_secret()),
            cancel.clone(),
        )
        .await?;
        let body = bounded(response, self.max_extracted_content_bytes, cancel).await?;
        String::from_utf8(body).map_err(|_| unavailable())
    }

    pub async fn delete(
        &self,
        credential: &SecretString,
        file_id: &SecretString,
        cancel: CancellationToken,
    ) -> AppResult<()> {
        validate_file_id(file_id.expose_secret())?;
        let request = self
            .client
            .delete(self.file_url(file_id, "")?)
            .bearer_auth(credential.expose_secret());
        let response = execute_allow_not_found(request, cancel).await?;
        if response.status().is_success() || response.status() == StatusCode::NOT_FOUND {
            Ok(())
        } else {
            Err(unavailable())
        }
    }

    fn url(&self, path: &str) -> AppResult<Url> {
        self.origin.join(path).map_err(|_| invalid())
    }
    fn file_url(&self, id: &SecretString, suffix: &str) -> AppResult<Url> {
        validate_file_id(id.expose_secret())?;
        self.url(&format!(
            "files/{}{}",
            id.expose_secret(),
            if suffix.is_empty() {
                String::new()
            } else {
                format!("/{suffix}")
            }
        ))
    }
}

#[derive(Deserialize)]
struct FileEnvelope {
    id: String,
    status: Option<String>,
}

async fn execute(
    builder: reqwest::RequestBuilder,
    cancel: CancellationToken,
) -> AppResult<reqwest::Response> {
    let response = execute_allow_not_found(builder, cancel).await?;
    if response.status().is_success() {
        return Ok(response);
    }
    let status = response.status();
    let body = bounded(response, 1024 * 1024, CancellationToken::new())
        .await
        .unwrap_or_default();
    Err(AiError::from_http(status, &body).into_app_error())
}

async fn execute_allow_not_found(
    builder: reqwest::RequestBuilder,
    cancel: CancellationToken,
) -> AppResult<reqwest::Response> {
    if cancel.is_cancelled() {
        return Err(cancelled());
    }
    tokio::select! {
        _ = cancel.cancelled() => Err(cancelled()),
        result = builder.send() => result.map_err(|error| AiError::from_reqwest(&error).into_app_error()),
    }
}

async fn bounded(
    response: reqwest::Response,
    limit: usize,
    cancel: CancellationToken,
) -> AppResult<Vec<u8>> {
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return Err(AppError::new(AppErrorCode::ContextTooLarge));
    }
    let mut stream = response.bytes_stream();
    let mut out = Vec::new();
    loop {
        let next = tokio::select! { _ = cancel.cancelled() => return Err(cancelled()), item = stream.next() => item };
        let Some(chunk) = next else { break };
        let chunk = chunk.map_err(|_| unavailable())?;
        if out
            .len()
            .checked_add(chunk.len())
            .is_none_or(|size| size > limit)
        {
            return Err(AppError::new(AppErrorCode::ContextTooLarge));
        }
        out.extend_from_slice(&chunk);
    }
    Ok(out)
}

fn validate_file_id(value: &str) -> AppResult<()> {
    if value.is_empty()
        || value.len() > 4096
        || !value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.'))
    {
        Err(invalid())
    } else {
        Ok(())
    }
}
fn invalid() -> AppError {
    AppError::new(AppErrorCode::InvalidInput)
}
fn unavailable() -> AppError {
    AppError::provider_failure(
        AppErrorCode::ProviderUnavailable,
        "Kimi Files API protocol failure",
    )
}
fn cancelled() -> AppError {
    AppError::new(AppErrorCode::ImportCancelled)
}
