use std::fmt;

use reqwest::StatusCode;
use serde::Deserialize;

use crate::{
    domain::ProviderKind,
    errors::{AppError, AppErrorCode},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AiErrorKind {
    InvalidInput,
    NetworkOffline,
    InvalidApiKey,
    ModelNotFound,
    ProviderPermissionDenied,
    ProviderRegionRestricted,
    RateLimited,
    InsufficientQuota,
    ContextTooLarge,
    ProviderRefused,
    ProviderUnavailable,
    Cancelled,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct AiError {
    kind: AiErrorKind,
}

impl AiError {
    pub const fn kind(&self) -> AiErrorKind {
        self.kind
    }

    pub const fn app_code(&self) -> AppErrorCode {
        match self.kind {
            AiErrorKind::InvalidInput => AppErrorCode::InvalidInput,
            AiErrorKind::NetworkOffline => AppErrorCode::NetworkOffline,
            AiErrorKind::InvalidApiKey => AppErrorCode::InvalidApiKey,
            AiErrorKind::ModelNotFound => AppErrorCode::ModelNotFound,
            AiErrorKind::ProviderPermissionDenied => AppErrorCode::ProviderPermissionDenied,
            AiErrorKind::ProviderRegionRestricted => AppErrorCode::ProviderRegionRestricted,
            AiErrorKind::RateLimited => AppErrorCode::RateLimited,
            AiErrorKind::InsufficientQuota => AppErrorCode::InsufficientQuota,
            AiErrorKind::ContextTooLarge => AppErrorCode::ContextTooLarge,
            AiErrorKind::ProviderRefused => AppErrorCode::ProviderRefused,
            AiErrorKind::ProviderUnavailable => AppErrorCode::ProviderUnavailable,
            AiErrorKind::Cancelled => AppErrorCode::ImportCancelled,
        }
    }

    pub fn into_app_error(self) -> AppError {
        match self.kind {
            AiErrorKind::InvalidInput | AiErrorKind::Cancelled => AppError::new(self.app_code()),
            _ => AppError::provider_failure(self.app_code(), self.kind.diagnostic_label()),
        }
    }

    pub const fn invalid_input() -> Self {
        Self::new(AiErrorKind::InvalidInput)
    }

    pub const fn network_offline() -> Self {
        Self::new(AiErrorKind::NetworkOffline)
    }

    pub const fn invalid_api_key() -> Self {
        Self::new(AiErrorKind::InvalidApiKey)
    }

    pub const fn model_not_found() -> Self {
        Self::new(AiErrorKind::ModelNotFound)
    }

    pub const fn permission_denied() -> Self {
        Self::new(AiErrorKind::ProviderPermissionDenied)
    }

    pub const fn region_restricted() -> Self {
        Self::new(AiErrorKind::ProviderRegionRestricted)
    }

    pub const fn rate_limited() -> Self {
        Self::new(AiErrorKind::RateLimited)
    }

    pub const fn insufficient_quota() -> Self {
        Self::new(AiErrorKind::InsufficientQuota)
    }

    pub const fn context_too_large() -> Self {
        Self::new(AiErrorKind::ContextTooLarge)
    }

    pub const fn refused() -> Self {
        Self::new(AiErrorKind::ProviderRefused)
    }

    pub const fn provider_unavailable() -> Self {
        Self::new(AiErrorKind::ProviderUnavailable)
    }

    pub const fn cancelled() -> Self {
        Self::new(AiErrorKind::Cancelled)
    }

    pub const fn malformed_event() -> Self {
        Self::provider_unavailable()
    }

    pub const fn unexpected_eof() -> Self {
        Self::provider_unavailable()
    }

    pub(crate) fn from_reqwest(error: &reqwest::Error) -> Self {
        if error.is_connect() {
            Self::network_offline()
        } else {
            Self::provider_unavailable()
        }
    }

    pub(crate) fn from_http(status: StatusCode, body: &[u8]) -> Self {
        let fields = serde_json::from_slice::<VendorErrorEnvelope>(body)
            .ok()
            .map(VendorErrorEnvelope::normalized_fields)
            .unwrap_or_default();

        Self::from_http_fields(status, &fields)
    }

    pub(crate) fn from_provider_http(
        provider: &ProviderKind,
        status: StatusCode,
        body: &[u8],
    ) -> Self {
        let fields = serde_json::from_slice::<VendorErrorEnvelope>(body)
            .ok()
            .map(VendorErrorEnvelope::normalized_fields)
            .unwrap_or_default();

        if matches!(provider, ProviderKind::Gemini) && status == StatusCode::TOO_MANY_REQUESTS {
            let kind = if contains_any(
                &fields,
                &[
                    "insufficient_quota",
                    "quota_exhausted",
                    "billing",
                    "payment_required",
                    "credit_balance",
                ],
            ) {
                AiErrorKind::InsufficientQuota
            } else if contains_any(
                &fields,
                &["rate_limit", "too_many_requests", "rate_limited"],
            ) {
                AiErrorKind::RateLimited
            } else {
                AiErrorKind::ProviderUnavailable
            };
            return Self::new(kind);
        }

        Self::from_http_fields(status, &fields)
    }

    fn from_http_fields(status: StatusCode, fields: &str) -> Self {
        let kind = if contains_any(
            fields,
            &["invalid_api_key", "authentication", "unauthorized"],
        ) || status == StatusCode::UNAUTHORIZED
        {
            AiErrorKind::InvalidApiKey
        } else if contains_any(
            fields,
            &[
                "context_length",
                "context_window",
                "context_too_large",
                "token_limit",
            ],
        ) || status == StatusCode::PAYLOAD_TOO_LARGE
        {
            AiErrorKind::ContextTooLarge
        } else if contains_any(
            fields,
            &[
                "model_not_found",
                "model_not_exist",
                "unknown_model",
                "invalid_model",
            ],
        ) || status == StatusCode::NOT_FOUND
        {
            AiErrorKind::ModelNotFound
        } else if contains_any(
            fields,
            &[
                "region_restricted",
                "regional_restriction",
                "unsupported_country",
                "location_restricted",
            ],
        ) {
            AiErrorKind::ProviderRegionRestricted
        } else if contains_any(
            fields,
            &[
                "insufficient_quota",
                "quota_exhausted",
                "billing",
                "payment_required",
                "credit_balance",
            ],
        ) || status == StatusCode::PAYMENT_REQUIRED
        {
            AiErrorKind::InsufficientQuota
        } else if contains_any(fields, &["rate_limit", "too_many_requests"])
            || status == StatusCode::TOO_MANY_REQUESTS
        {
            AiErrorKind::RateLimited
        } else if contains_any(
            fields,
            &["refused", "refusal", "content_policy", "safety_rejection"],
        ) {
            AiErrorKind::ProviderRefused
        } else if contains_any(fields, &["permission_denied", "forbidden", "access_denied"])
            || status == StatusCode::FORBIDDEN
        {
            AiErrorKind::ProviderPermissionDenied
        } else {
            AiErrorKind::ProviderUnavailable
        };

        Self::new(kind)
    }

    const fn new(kind: AiErrorKind) -> Self {
        Self { kind }
    }
}

impl AiErrorKind {
    const fn diagnostic_label(self) -> &'static str {
        match self {
            Self::InvalidInput => "provider input rejected locally",
            Self::NetworkOffline => "provider connection failed",
            Self::InvalidApiKey => "provider authentication failed",
            Self::ModelNotFound => "provider model was unavailable",
            Self::ProviderPermissionDenied => "provider permission was denied",
            Self::ProviderRegionRestricted => "provider region restriction was reported",
            Self::RateLimited => "provider rate limit was reported",
            Self::InsufficientQuota => "provider quota exhaustion was reported",
            Self::ContextTooLarge => "provider context limit was exceeded",
            Self::ProviderRefused => "provider refused the request",
            Self::ProviderUnavailable => "provider protocol or availability failure",
            Self::Cancelled => "provider operation was cancelled",
        }
    }
}

impl fmt::Debug for AiError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AiError")
            .field("kind", &self.kind)
            .finish()
    }
}

impl fmt::Display for AiError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.kind.diagnostic_label())
    }
}

impl std::error::Error for AiError {}

#[derive(Default, Deserialize)]
struct VendorErrorEnvelope {
    error: Option<VendorErrorFields>,
    code: Option<VendorScalar>,
    #[serde(rename = "type")]
    kind: Option<VendorScalar>,
    status: Option<VendorScalar>,
}

impl VendorErrorEnvelope {
    fn normalized_fields(self) -> String {
        let nested = self.error.unwrap_or_default();
        [
            self.code,
            self.kind,
            self.status,
            nested.code,
            nested.kind,
            nested.status,
        ]
        .into_iter()
        .flatten()
        .map(VendorScalar::into_string)
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
    }
}

#[derive(Default, Deserialize)]
struct VendorErrorFields {
    code: Option<VendorScalar>,
    #[serde(rename = "type")]
    kind: Option<VendorScalar>,
    status: Option<VendorScalar>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum VendorScalar {
    String(String),
    Signed(i64),
    Unsigned(u64),
}

impl VendorScalar {
    fn into_string(self) -> String {
        match self {
            Self::String(value) => value,
            Self::Signed(value) => value.to_string(),
            Self::Unsigned(value) => value.to_string(),
        }
    }
}

fn contains_any(fields: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| fields.contains(needle))
}
