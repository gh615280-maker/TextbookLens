use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use super::NormalizedRect;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "provenance.ts")]
pub enum ContentSource {
    LocalText,
    AiTranscribed,
    AiDescription,
    UserCorrected,
}

impl ContentSource {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LocalText => "local_text",
            Self::AiTranscribed => "ai_transcribed",
            Self::AiDescription => "ai_description",
            Self::UserCorrected => "user_corrected",
        }
    }

    pub fn from_database(value: &str) -> Option<Self> {
        match value {
            "local_text" => Some(Self::LocalText),
            "ai_transcribed" => Some(Self::AiTranscribed),
            "ai_description" => Some(Self::AiDescription),
            "user_corrected" => Some(Self::UserCorrected),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "provenance.ts")]
pub enum IndexPageBlockKind {
    Title,
    Paragraph,
    List,
    Table,
    Caption,
    Formula,
    Figure,
    Transcript,
}

impl IndexPageBlockKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Title => "title",
            Self::Paragraph => "paragraph",
            Self::List => "list",
            Self::Table => "table",
            Self::Caption => "caption",
            Self::Formula => "formula",
            Self::Figure => "figure",
            Self::Transcript => "transcript",
        }
    }

    pub fn from_database(value: &str) -> Option<Self> {
        match value {
            "title" => Some(Self::Title),
            "paragraph" => Some(Self::Paragraph),
            "list" => Some(Self::List),
            "table" => Some(Self::Table),
            "caption" => Some(Self::Caption),
            "formula" => Some(Self::Formula),
            "figure" => Some(Self::Figure),
            "transcript" => Some(Self::Transcript),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "provenance.ts")]
pub enum IndexCorrectionValueKind {
    Text,
    Latex,
}

impl IndexCorrectionValueKind {
    pub fn from_database(value: &str) -> Option<Self> {
        match value {
            "text" => Some(Self::Text),
            "latex" => Some(Self::Latex),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "provenance.ts")]
pub enum IndexCorrectionConflictState {
    Active,
    Conflict,
}

impl IndexCorrectionConflictState {
    pub fn from_database(value: &str) -> Option<Self> {
        match value {
            "active" => Some(Self::Active),
            "conflict" => Some(Self::Conflict),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "provenance.ts")]
pub struct IndexTableCellDto {
    pub row: u32,
    pub column: u32,
    pub row_span: u32,
    pub column_span: u32,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "provenance.ts")]
pub struct IndexPageBlockReviewDto {
    pub id: Uuid,
    pub ordinal: u32,
    pub kind: IndexPageBlockKind,
    pub source: ContentSource,
    pub plain_text: Option<String>,
    pub latex: Option<String>,
    pub table_cells: Option<Vec<IndexTableCellDto>>,
    pub visual_description: Option<String>,
    pub bounds: Option<NormalizedRect>,
    pub content_version: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "provenance.ts")]
pub struct IndexCorrectionReviewDto {
    pub id: Uuid,
    pub target_block_id: Uuid,
    pub region: Option<NormalizedRect>,
    pub value_kind: IndexCorrectionValueKind,
    pub original_value: String,
    pub corrected_value: String,
    pub conflict_state: IndexCorrectionConflictState,
    pub revision: u32,
    pub updated_at: DateTime<Utc>,
}
