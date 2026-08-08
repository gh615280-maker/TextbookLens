use std::{fmt, sync::Arc};

#[cfg(test)]
use std::time::Duration;

use chrono::{DateTime, Utc};
use secrecy::SecretString;
use sqlx::SqlitePool;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::{
    error::{AiError, AiErrorKind},
    provider::{
        AiProvider, ProviderStream, UnifiedChatRequest, validate_credential, validate_model_id,
    },
    providers::{
        anthropic::AnthropicProvider, deepseek::DeepSeekProvider, gemini::GeminiProvider,
        kimi::KimiProvider, openai::OpenAiProvider,
    },
    registry::ProviderCapabilityRegistry,
};
use crate::{
    credentials::CredentialStore,
    db::providers,
    domain::{
        AiOperation, CapabilitySupport, KimiApiRegion, ProviderKind, ProviderProfileSummary,
        RemoteCleanupHandle, StructuredAnalysisOutcome, StructuredPageRequest, ValidationResult,
    },
    errors::{AppError, AppErrorCode, AppResult},
};

pub struct ProviderRuntime {
    credential_store: Arc<dyn CredentialStore>,
    registry: ProviderCapabilityRegistry,
    #[cfg(test)]
    loopback_origin: Option<String>,
    #[cfg(test)]
    kimi_loopback_origins: Option<(String, String)>,
    #[cfg(test)]
    kimi_loopback_timeout: Option<Duration>,
}

pub struct LoadedProvider {
    profile: ProviderProfileSummary,
    operation: AiOperation,
    credential: SecretString,
    adapter: Box<dyn AiProvider>,
}

pub(crate) struct ValidatedProviderCredential {
    pub kind: ProviderKind,
    pub model_id: String,
    pub context_window_tokens: u32,
    pub credential: SecretString,
    pub validated_at: DateTime<Utc>,
    pub kimi_api_region: Option<KimiApiRegion>,
}

impl ProviderRuntime {
    pub fn new(
        credential_store: Arc<dyn CredentialStore>,
        registry: ProviderCapabilityRegistry,
    ) -> Self {
        Self {
            credential_store,
            registry,
            #[cfg(test)]
            loopback_origin: None,
            #[cfg(test)]
            kimi_loopback_origins: None,
            #[cfg(test)]
            kimi_loopback_timeout: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn new_for_test(
        credential_store: Arc<dyn CredentialStore>,
        registry: ProviderCapabilityRegistry,
        loopback_origin: &str,
    ) -> AppResult<Self> {
        OpenAiProvider::new_for_test(loopback_origin).map_err(|error| error.into_app_error())?;
        Ok(Self {
            credential_store,
            registry,
            loopback_origin: Some(loopback_origin.to_owned()),
            kimi_loopback_origins: Some((loopback_origin.to_owned(), loopback_origin.to_owned())),
            kimi_loopback_timeout: None,
        })
    }

    #[cfg(test)]
    pub(crate) fn new_for_test_with_kimi_origins(
        credential_store: Arc<dyn CredentialStore>,
        registry: ProviderCapabilityRegistry,
        cn_origin: &str,
        international_origin: &str,
    ) -> AppResult<Self> {
        KimiProvider::new_for_test(cn_origin).map_err(AiError::into_app_error)?;
        KimiProvider::new_for_test(international_origin).map_err(AiError::into_app_error)?;
        Ok(Self {
            credential_store,
            registry,
            loopback_origin: Some(cn_origin.to_owned()),
            kimi_loopback_origins: Some((cn_origin.to_owned(), international_origin.to_owned())),
            kimi_loopback_timeout: None,
        })
    }

    #[cfg(test)]
    pub(crate) fn new_for_test_with_kimi_origins_and_timeout(
        credential_store: Arc<dyn CredentialStore>,
        registry: ProviderCapabilityRegistry,
        cn_origin: &str,
        international_origin: &str,
        timeout: Duration,
    ) -> AppResult<Self> {
        KimiProvider::new_for_test_with_timeout(cn_origin, timeout)
            .map_err(AiError::into_app_error)?;
        KimiProvider::new_for_test_with_timeout(international_origin, timeout)
            .map_err(AiError::into_app_error)?;
        Ok(Self {
            credential_store,
            registry,
            loopback_origin: Some(cn_origin.to_owned()),
            kimi_loopback_origins: Some((cn_origin.to_owned(), international_origin.to_owned())),
            kimi_loopback_timeout: Some(timeout),
        })
    }

    pub async fn load(
        &self,
        pool: &SqlitePool,
        profile_id: Uuid,
        operation: AiOperation,
    ) -> AppResult<LoadedProvider> {
        self.load_with_kimi_region(pool, profile_id, operation, None)
            .await
    }

    pub(crate) async fn load_for_remote_cleanup(
        &self,
        pool: &SqlitePool,
        profile_id: Uuid,
        operation: AiOperation,
        handle: &RemoteCleanupHandle,
    ) -> AppResult<LoadedProvider> {
        self.load_with_kimi_region(pool, profile_id, operation, handle.kimi_api_region())
            .await
    }

    async fn load_with_kimi_region(
        &self,
        pool: &SqlitePool,
        profile_id: Uuid,
        operation: AiOperation,
        region_override: Option<KimiApiRegion>,
    ) -> AppResult<LoadedProvider> {
        let mut profile = providers::load_provider_profile_metadata(pool, profile_id).await?;
        self.ensure_model_support(&profile.kind, &profile.model_id, operation)?;
        if profile.kind != ProviderKind::Kimi && region_override.is_some() {
            return Err(AppError::new(AppErrorCode::InvalidInput));
        }
        let region = region_override.or(profile.kimi_api_region);
        let adapter = self.adapter(&profile.kind, region)?;
        if adapter.kind() != profile.kind {
            return Err(AppError::new(AppErrorCode::ProviderUnavailable));
        }
        let credential = self
            .credential_store
            .get(&providers::credential_key(profile.id))
            .await?;
        profile.credential_status = providers::CredentialStatus::Available;
        Ok(LoadedProvider {
            profile,
            operation,
            credential,
            adapter,
        })
    }

    pub(crate) async fn validate_candidate(
        &self,
        kind: ProviderKind,
        requested_model: Option<&str>,
        credential: SecretString,
        operation: AiOperation,
    ) -> AppResult<ValidatedProviderCredential> {
        validate_credential(&credential).map_err(|error| error.into_app_error())?;
        let model_id = self.resolve_model(&kind, requested_model)?;
        self.ensure_model_support(&kind, &model_id, operation)?;
        let (validation, kimi_api_region) = if kind == ProviderKind::Kimi {
            self.validate_kimi_candidate(&credential, &model_id).await?
        } else {
            let adapter = self.adapter(&kind, None)?;
            if adapter.kind() != kind {
                return Err(AppError::new(AppErrorCode::ProviderUnavailable));
            }
            (
                adapter
                    .validate(&credential, &model_id)
                    .await
                    .map_err(AiError::into_app_error)?,
                None,
            )
        };
        if validation.model != model_id || validation.context_window_tokens == 0 {
            return Err(AppError::new(AppErrorCode::ProviderUnavailable));
        }
        self.ensure_model_support(&kind, &validation.model, operation)?;
        Ok(ValidatedProviderCredential {
            kind,
            model_id: validation.model,
            context_window_tokens: validation.context_window_tokens,
            credential,
            validated_at: Utc::now(),
            kimi_api_region,
        })
    }

    async fn validate_kimi_candidate(
        &self,
        credential: &SecretString,
        model_id: &str,
    ) -> AppResult<(ValidationResult, Option<KimiApiRegion>)> {
        let cn = self.adapter(&ProviderKind::Kimi, Some(KimiApiRegion::Cn))?;
        match cn.validate(credential, model_id).await {
            Ok(validation) => Ok((validation, Some(KimiApiRegion::Cn))),
            Err(error) if should_probe_alternate_kimi_region(&error) => {
                let international =
                    self.adapter(&ProviderKind::Kimi, Some(KimiApiRegion::International))?;
                international
                    .validate(credential, model_id)
                    .await
                    .map(|validation| (validation, Some(KimiApiRegion::International)))
                    .map_err(AiError::into_app_error)
            }
            Err(error) => Err(error.into_app_error()),
        }
    }

    fn resolve_model(
        &self,
        kind: &ProviderKind,
        requested_model: Option<&str>,
    ) -> AppResult<String> {
        let provider = self
            .registry
            .capabilities()
            .iter()
            .find(|provider| &provider.kind == kind)
            .ok_or_else(|| AppError::new(AppErrorCode::InvalidInput))?;
        let model_id = requested_model.unwrap_or(&provider.default_model);
        validate_model_id(model_id).map_err(|error| error.into_app_error())?;
        if !provider.models.iter().any(|model| model.id == model_id) {
            return Err(AppError::new(AppErrorCode::ModelNotFound));
        }
        Ok(model_id.to_owned())
    }

    fn ensure_model_support(
        &self,
        kind: &ProviderKind,
        model_id: &str,
        operation: AiOperation,
    ) -> AppResult<()> {
        let owns_model = self
            .registry
            .capabilities()
            .iter()
            .find(|provider| &provider.kind == kind)
            .is_some_and(|provider| provider.models.iter().any(|model| model.id == model_id));
        if !owns_model {
            return Err(AppError::new(AppErrorCode::ModelNotFound));
        }
        if self.registry.operation_support(kind, model_id, operation)
            != CapabilitySupport::Supported
        {
            return Err(AppError::unsupported_provider_capability());
        }
        Ok(())
    }

    fn adapter(
        &self,
        kind: &ProviderKind,
        kimi_api_region: Option<KimiApiRegion>,
    ) -> AppResult<Box<dyn AiProvider>> {
        #[cfg(test)]
        if let Some(origin) = self.loopback_origin.as_deref() {
            return match kind {
                ProviderKind::OpenAi => OpenAiProvider::new_for_test(origin)
                    .map(|provider| Box::new(provider) as Box<dyn AiProvider>),
                ProviderKind::Gemini => GeminiProvider::new_for_test(origin)
                    .map(|provider| Box::new(provider) as Box<dyn AiProvider>),
                ProviderKind::Anthropic => AnthropicProvider::new_for_test(origin)
                    .map(|provider| Box::new(provider) as Box<dyn AiProvider>),
                ProviderKind::DeepSeek => DeepSeekProvider::new_for_test(origin)
                    .map(|provider| Box::new(provider) as Box<dyn AiProvider>),
                ProviderKind::Kimi => {
                    let region = kimi_api_region
                        .ok_or_else(|| AppError::new(AppErrorCode::DatabaseError))?;
                    let origins = self
                        .kimi_loopback_origins
                        .as_ref()
                        .ok_or_else(|| AppError::new(AppErrorCode::DatabaseError))?;
                    let origin = match region {
                        KimiApiRegion::Cn => origins.0.as_str(),
                        KimiApiRegion::International => origins.1.as_str(),
                    };
                    match self.kimi_loopback_timeout {
                        Some(timeout) => KimiProvider::new_for_test_with_timeout(origin, timeout),
                        None => KimiProvider::new_for_test(origin),
                    }
                }
                .map(|provider| Box::new(provider) as Box<dyn AiProvider>),
            }
            .map_err(|error| error.into_app_error());
        }

        match kind {
            ProviderKind::OpenAi => {
                OpenAiProvider::new().map(|provider| Box::new(provider) as Box<dyn AiProvider>)
            }
            ProviderKind::Gemini => {
                GeminiProvider::new().map(|provider| Box::new(provider) as Box<dyn AiProvider>)
            }
            ProviderKind::Anthropic => {
                AnthropicProvider::new().map(|provider| Box::new(provider) as Box<dyn AiProvider>)
            }
            ProviderKind::DeepSeek => {
                DeepSeekProvider::new().map(|provider| Box::new(provider) as Box<dyn AiProvider>)
            }
            ProviderKind::Kimi => KimiProvider::new_for_region(
                kimi_api_region.ok_or_else(|| AppError::new(AppErrorCode::DatabaseError))?,
            )
            .map(|provider| Box::new(provider) as Box<dyn AiProvider>),
        }
        .map_err(|error| error.into_app_error())
    }
}

fn should_probe_alternate_kimi_region(error: &AiError) -> bool {
    matches!(
        error.kind(),
        AiErrorKind::InvalidApiKey | AiErrorKind::ProviderRegionRestricted
    )
}

impl LoadedProvider {
    pub fn profile(&self) -> &ProviderProfileSummary {
        &self.profile
    }

    pub const fn operation(&self) -> AiOperation {
        self.operation
    }

    pub fn provider_kind(&self) -> ProviderKind {
        self.adapter.kind()
    }

    pub async fn revalidate(&self) -> AppResult<ValidationResult> {
        self.adapter
            .validate(&self.credential, &self.profile.model_id)
            .await
            .map_err(|error| error.into_app_error())
    }

    /// Streams a text-learning request using the already captured profile,
    /// credential, and provider adapter. This deliberately exposes no generic
    /// execution or credential access outside the runtime boundary.
    pub async fn stream_text_learning(
        &self,
        request: UnifiedChatRequest,
        cancel: CancellationToken,
    ) -> AppResult<ProviderStream> {
        if self.operation != AiOperation::TextLearning || request.model != self.profile.model_id {
            return Err(AppError::new(AppErrorCode::InvalidInput));
        }
        self.adapter
            .stream_chat(&self.credential, request, cancel)
            .await
            .map_err(|error| error.into_app_error())
    }

    /// Streams a vision-learning request through the same captured credential boundary.
    pub async fn stream_vision_learning(
        &self,
        request: crate::domain::UnifiedVisionRequest,
        cancel: CancellationToken,
    ) -> AppResult<ProviderStream> {
        if self.operation != AiOperation::VisionLearning
            || request.text.model != self.profile.model_id
        {
            return Err(AppError::new(AppErrorCode::InvalidInput));
        }
        self.adapter
            .stream_vision(&self.credential, request, cancel)
            .await
    }

    /// Executes the one structured operation captured by `load`; adapter and
    /// credential remain private to this boundary.
    pub async fn analyze_pages(
        &self,
        request: StructuredPageRequest,
        cancel: CancellationToken,
    ) -> AppResult<StructuredAnalysisOutcome> {
        if self.operation != AiOperation::StructuredPageAnalysis
            || request.model != self.profile.model_id
        {
            return Err(AppError::new(AppErrorCode::InvalidInput));
        }
        if cancel.is_cancelled() {
            return Err(AppError::new(AppErrorCode::ImportCancelled));
        }
        self.adapter
            .analyze_pages(&self.credential, request, cancel)
            .await
    }

    /// Deletes only a handle owned by the captured structured provider.
    pub async fn cleanup_remote_resource(
        &self,
        handle: &RemoteCleanupHandle,
        cancel: CancellationToken,
    ) -> AppResult<()> {
        if self.operation != AiOperation::StructuredPageAnalysis
            || handle.provider() != &self.profile.kind
        {
            return Err(AppError::new(AppErrorCode::InvalidInput));
        }
        if cancel.is_cancelled() {
            return Err(AppError::new(AppErrorCode::ImportCancelled));
        }
        self.adapter
            .cleanup_remote_resource(&self.credential, handle, cancel)
            .await
    }
}

impl fmt::Debug for ProviderRuntime {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderRuntime")
            .field("provider_count", &self.registry.capabilities().len())
            .finish_non_exhaustive()
    }
}

impl fmt::Debug for LoadedProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LoadedProvider")
            .field("profile_id", &self.profile.id)
            .field("kind", &self.profile.kind)
            .field("operation", &self.operation)
            .finish_non_exhaustive()
    }
}

impl fmt::Debug for ValidatedProviderCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ValidatedProviderCredential")
            .field("kind", &self.kind)
            .field("model_id", &self.model_id)
            .field("context_window_tokens", &self.context_window_tokens)
            .field("validated_at", &self.validated_at)
            .field("kimi_api_region", &self.kimi_api_region)
            .finish_non_exhaustive()
    }
}
