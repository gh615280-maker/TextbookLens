use std::{fmt, io};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

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
    fn user_message(self) -> &'static str {
        match self {
            Self::ImportCancelled => "上次导入被应用退出中断，请重试",
            Self::DatabaseError => "本地数据库暂时不可用。",
            Self::LocalIoError => "本地文件操作失败。",
            Self::InvalidInput => "输入内容无效。",
            Self::NotFound => "未找到所需内容。",
            Self::BookNotReady => "教材尚未准备完成。",
            Self::RequestConflict => "该请求正在处理中。",
            _ => "操作未能完成。",
        }
    }

    fn next_step(self) -> &'static str {
        match self {
            Self::ImportCancelled => "重新开始导入。",
            Self::DatabaseError => "重启应用后重试；若问题持续，请导出诊断信息。",
            Self::LocalIoError => "检查磁盘空间和文件访问权限后重试。",
            _ => "检查输入后重试。",
        }
    }
}

#[derive(Debug)]
pub struct AppError {
    pub code: AppErrorCode,
    pub message: String,
    pub next_step: String,
    diagnostic_detail: Option<String>,
}

impl AppError {
    pub fn new(code: AppErrorCode) -> Self {
        Self {
            code,
            message: code.user_message().to_owned(),
            next_step: code.next_step().to_owned(),
            diagnostic_detail: None,
        }
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

    pub fn diagnostic_detail(&self) -> Option<&str> {
        self.diagnostic_detail.as_deref()
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
    pub code: AppErrorCode,
    pub message: String,
    pub next_step: String,
    pub diagnostic_id: Option<Uuid>,
}

impl From<AppError> for AppErrorDto {
    fn from(error: AppError) -> Self {
        Self {
            code: error.code,
            message: error.message,
            next_step: error.next_step,
            diagnostic_id: error.diagnostic_detail.map(|_| Uuid::new_v4()),
        }
    }
}
