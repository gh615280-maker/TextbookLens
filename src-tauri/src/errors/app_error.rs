use std::{fmt, io};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::redaction::{SafeDiagnostic, redact};

pub const UNSUPPORTED_PROVIDER_CAPABILITY_CODE: &str = "UNSUPPORTED_PROVIDER_CAPABILITY";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AppErrorCode {
    InvalidApiKey,
    ModelNotFound,
    ProviderPermissionDenied,
    ProviderRegionRestricted,
    RateLimited,
    InsufficientQuota,
    ContextTooLarge,
    NetworkOffline,
    ProviderUnavailable,
    ProviderRefused,
    UnsupportedFileType,
    FileCorrupted,
    FileEncryptedOrDrm,
    NoExtractableText,
    ImportCancelled,
    DatabaseError,
    CredentialStoreError,
    AnchorNotFound,
    InvalidInput,
    NotFound,
    BookNotReady,
    RequestConflict,
    LocalIoError,
}

impl AppErrorCode {
    pub const fn user_message(self) -> &'static str {
        match self {
            Self::InvalidApiKey => "API Key 无效。",
            Self::ModelNotFound => "未找到指定的模型。",
            Self::ProviderPermissionDenied => "服务商拒绝了当前凭据的访问。",
            Self::ProviderRegionRestricted => "当前地区无法使用该服务。",
            Self::RateLimited => "请求过于频繁。",
            Self::InsufficientQuota => "服务商账户额度不足。",
            Self::ContextTooLarge => "请求内容超出模型上下文限制。",
            Self::NetworkOffline => "当前网络不可用。",
            Self::ProviderUnavailable => "AI 服务暂时不可用。",
            Self::ProviderRefused => "AI 服务拒绝了该请求。",
            Self::UnsupportedFileType => "不支持该文件类型。",
            Self::FileCorrupted => "文件已损坏或无法读取。",
            Self::FileEncryptedOrDrm => "文件受密码或 DRM 保护。",
            Self::NoExtractableText => "文件中没有可提取的文本。",
            Self::ImportCancelled => "上次导入被应用退出中断，请重试",
            Self::DatabaseError => "本地数据库暂时不可用。",
            Self::CredentialStoreError => "无法访问 Windows 凭据管理器。",
            Self::AnchorNotFound => "无法在教材中恢复该位置。",
            Self::LocalIoError => "本地文件操作失败。",
            Self::InvalidInput => "输入内容无效。",
            Self::NotFound => "未找到所需内容。",
            Self::BookNotReady => "教材尚未准备完成。",
            Self::RequestConflict => "该请求正在处理中。",
        }
    }

    pub const fn next_step(self) -> &'static str {
        match self {
            Self::InvalidApiKey => "检查 API Key 后重新保存。",
            Self::ModelNotFound => "选择服务商支持的模型后重试。",
            Self::ProviderPermissionDenied => "检查账户权限或更换 API Key。",
            Self::ProviderRegionRestricted => "改用当前地区可用的服务商。",
            Self::RateLimited => "稍后重试。",
            Self::InsufficientQuota => "检查服务商账户余额或配额后重试。",
            Self::ContextTooLarge => "缩短选区或改用更大上下文的模型。",
            Self::NetworkOffline => "恢复网络连接后重试。",
            Self::ProviderUnavailable => "稍后重试或切换服务商。",
            Self::ProviderRefused => "调整问题内容后重试。",
            Self::UnsupportedFileType => "请选择 PDF、EPUB 或 DOCX 文件。",
            Self::FileCorrupted => "重新获取文件后再导入。",
            Self::FileEncryptedOrDrm => "请使用未加密且无 DRM 的文件。",
            Self::NoExtractableText => "请选择包含可选择文本的文件。",
            Self::ImportCancelled => "重新开始导入。",
            Self::DatabaseError => "重启应用后重试；若问题持续，请导出诊断信息。",
            Self::CredentialStoreError => "重新登录 Windows 后重试。",
            Self::AnchorNotFound => "从教材目录重新定位该内容。",
            Self::LocalIoError => "检查磁盘空间和文件访问权限后重试。",
            Self::InvalidInput => "检查输入后重试。",
            Self::NotFound => "刷新页面后重试。",
            Self::BookNotReady => "等待导入完成后重试。",
            Self::RequestConflict => "等待当前请求完成或先取消它。",
        }
    }
}

#[derive(Debug)]
pub struct AppError {
    pub code: AppErrorCode,
    pub message: String,
    pub next_step: String,
    diagnostic_detail: Option<String>,
    stable_code_override: Option<&'static str>,
}

impl AppError {
    pub fn new(code: AppErrorCode) -> Self {
        Self {
            code,
            message: code.user_message().to_owned(),
            next_step: code.next_step().to_owned(),
            diagnostic_detail: None,
            stable_code_override: None,
        }
    }

    pub fn unsupported_provider_capability() -> Self {
        let mut result = Self::new(AppErrorCode::InvalidInput);
        result.message = "当前模型不支持该 AI 操作。".to_owned();
        result.next_step = "选择已明确支持该能力的模型后重试。".to_owned();
        result.stable_code_override = Some(UNSUPPORTED_PROVIDER_CAPABILITY_CODE);
        result
    }

    pub fn database(error: impl fmt::Display) -> Self {
        let mut result = Self::new(AppErrorCode::DatabaseError);
        result.diagnostic_detail = Some(error.to_string());
        result
    }

    pub fn local_io(error: impl fmt::Display) -> Self {
        let mut result = Self::new(AppErrorCode::LocalIoError);
        result.diagnostic_detail = Some(error.to_string());
        result
    }

    pub fn credential_store(error: impl fmt::Display) -> Self {
        let mut result = Self::new(AppErrorCode::CredentialStoreError);
        result.diagnostic_detail = Some(error.to_string());
        result
    }

    pub fn invalid_api_key(_secret: impl fmt::Display, _provider_body: impl fmt::Display) -> Self {
        let mut result = Self::new(AppErrorCode::InvalidApiKey);
        result.diagnostic_detail =
            Some("credential validation failed; sensitive inputs omitted".to_owned());
        result
    }

    pub(crate) fn provider_failure(code: AppErrorCode, diagnostic: &'static str) -> Self {
        let mut result = Self::new(code);
        result.diagnostic_detail = Some(diagnostic.to_owned());
        result
    }

    pub fn diagnostic_detail(&self) -> Option<&str> {
        self.diagnostic_detail.as_deref()
    }

    pub fn stable_code(&self) -> &'static str {
        self.stable_code_override
            .unwrap_or_else(|| self.code.stable_code())
    }
}

impl fmt::Display for AppError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.message)
    }
}

impl std::error::Error for AppError {}

impl From<sqlx::Error> for AppError {
    fn from(error: sqlx::Error) -> Self {
        Self::database(error)
    }
}

impl From<io::Error> for AppError {
    fn from(error: io::Error) -> Self {
        Self::local_io(error)
    }
}

pub type AppResult<T> = Result<T, AppError>;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppErrorDto {
    pub code: String,
    pub message: String,
    pub next_step: String,
    pub diagnostic_id: Option<Uuid>,
}

impl From<AppError> for AppErrorDto {
    fn from(error: AppError) -> Self {
        let stable_code = error.stable_code().to_owned();
        let diagnostic = error
            .diagnostic_detail
            .as_deref()
            .map(|detail| SafeDiagnostic::new(error.code, redact(detail)));

        if let Some(diagnostic) = &diagnostic {
            tracing::error!(
                diagnostic_id = %diagnostic.id,
                code = ?diagnostic.code,
                detail = %diagnostic.detail,
                "application operation failed"
            );
        }

        Self {
            code: stable_code,
            message: error.message,
            next_step: error.next_step,
            diagnostic_id: diagnostic.map(|diagnostic| diagnostic.id),
        }
    }
}

impl AppErrorCode {
    const fn stable_code(self) -> &'static str {
        match self {
            Self::InvalidApiKey => "INVALID_API_KEY",
            Self::ModelNotFound => "MODEL_NOT_FOUND",
            Self::ProviderPermissionDenied => "PROVIDER_PERMISSION_DENIED",
            Self::ProviderRegionRestricted => "PROVIDER_REGION_RESTRICTED",
            Self::RateLimited => "RATE_LIMITED",
            Self::InsufficientQuota => "INSUFFICIENT_QUOTA",
            Self::ContextTooLarge => "CONTEXT_TOO_LARGE",
            Self::NetworkOffline => "NETWORK_OFFLINE",
            Self::ProviderUnavailable => "PROVIDER_UNAVAILABLE",
            Self::ProviderRefused => "PROVIDER_REFUSED",
            Self::UnsupportedFileType => "UNSUPPORTED_FILE_TYPE",
            Self::FileCorrupted => "FILE_CORRUPTED",
            Self::FileEncryptedOrDrm => "FILE_ENCRYPTED_OR_DRM",
            Self::NoExtractableText => "NO_EXTRACTABLE_TEXT",
            Self::ImportCancelled => "IMPORT_CANCELLED",
            Self::DatabaseError => "DATABASE_ERROR",
            Self::CredentialStoreError => "CREDENTIAL_STORE_ERROR",
            Self::AnchorNotFound => "ANCHOR_NOT_FOUND",
            Self::InvalidInput => "INVALID_INPUT",
            Self::NotFound => "NOT_FOUND",
            Self::BookNotReady => "BOOK_NOT_READY",
            Self::RequestConflict => "REQUEST_CONFLICT",
            Self::LocalIoError => "LOCAL_IO_ERROR",
        }
    }
}
