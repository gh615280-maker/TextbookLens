use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "book.ts")]
pub enum BookFormat {
    Pdf,
    Epub,
    Docx,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "book.ts")]
pub enum ImportStatus {
    Queued,
    Copying,
    Parsing,
    Indexing,
    Ready,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "book.ts")]
pub enum ImportErrorStage {
    Copying,
    Parsing,
    Indexing,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "book.ts")]
pub struct BookSummary {
    pub id: Uuid,
    pub title: String,
    pub original_filename: String,
    pub author: Option<String>,
    pub language: Option<String>,
    pub format: BookFormat,
    pub import_status: ImportStatus,
    pub import_error_code: Option<String>,
    pub import_error_message: Option<String>,
    pub import_error_stage: Option<ImportErrorStage>,
    pub reading_progress: f64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub last_opened_at: Option<DateTime<Utc>>,
}
