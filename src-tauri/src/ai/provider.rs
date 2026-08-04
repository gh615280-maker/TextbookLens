use std::{pin::Pin, str::FromStr};

use async_trait::async_trait;
use futures_util::Stream;
use secrecy::{ExposeSecret, SecretString};
use tokio_util::sync::CancellationToken;

use super::error::AiError;
pub use crate::domain::{
    ProviderKind, RemoteCleanupHandle, StructuredAnalysisOutcome, StructuredPageRequest,
    UnifiedChatRequest, UnifiedMessage, UnifiedRole, UnifiedStreamEvent, UnifiedVisionRequest,
    ValidationResult,
};
use crate::errors::{AppError, AppResult};

pub const MAX_CREDENTIAL_BYTES: usize = 6_384;
pub const MAX_MODEL_ID_CHARS: usize = 256;
pub const MAX_DISPLAY_NAME_CHARS: usize = 80;

pub type ProviderStream = Pin<Box<dyn Stream<Item = Result<UnifiedStreamEvent, AiError>> + Send>>;

#[async_trait]
pub trait AiProvider: Send + Sync {
    fn kind(&self) -> ProviderKind;

    async fn validate(
        &self,
        credential: &SecretString,
        model: &str,
    ) -> Result<ValidationResult, AiError>;

    async fn stream_chat(
        &self,
        credential: &SecretString,
        request: UnifiedChatRequest,
        cancel: CancellationToken,
    ) -> Result<ProviderStream, AiError>;

    async fn stream_vision(
        &self,
        _credential: &SecretString,
        _request: UnifiedVisionRequest,
        _cancel: CancellationToken,
    ) -> AppResult<ProviderStream> {
        Err(unsupported_capability())
    }

    async fn analyze_pages(
        &self,
        _credential: &SecretString,
        _request: StructuredPageRequest,
        _cancel: CancellationToken,
    ) -> AppResult<StructuredAnalysisOutcome> {
        Err(unsupported_capability())
    }

    async fn cleanup_remote_resource(
        &self,
        _credential: &SecretString,
        _handle: &RemoteCleanupHandle,
        _cancel: CancellationToken,
    ) -> AppResult<()> {
        Err(unsupported_capability())
    }
}

fn unsupported_capability() -> AppError {
    AppError::unsupported_provider_capability()
}

pub fn validate_credential(credential: &SecretString) -> Result<(), AiError> {
    let length = credential.expose_secret().len();
    if !(1..=MAX_CREDENTIAL_BYTES).contains(&length) {
        return Err(AiError::invalid_input());
    }
    Ok(())
}

pub fn validate_model_id(model: &str) -> Result<(), AiError> {
    validate_trimmed_text(model, MAX_MODEL_ID_CHARS)
}

pub fn validate_display_name(display_name: &str) -> Result<(), AiError> {
    validate_trimmed_text(display_name, MAX_DISPLAY_NAME_CHARS)
}

fn validate_trimmed_text(value: &str, max_chars: usize) -> Result<(), AiError> {
    let length = value.chars().count();
    if value.trim() != value
        || !(1..=max_chars).contains(&length)
        || value.chars().any(char::is_control)
    {
        return Err(AiError::invalid_input());
    }
    Ok(())
}

impl FromStr for UnifiedRole {
    type Err = AiError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "user" => Ok(Self::User),
            "assistant" => Ok(Self::Assistant),
            _ => Err(AiError::invalid_input()),
        }
    }
}
