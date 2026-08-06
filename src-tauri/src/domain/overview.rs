use std::fmt;

use serde::Serialize;
use ts_rs::TS;
use uuid::Uuid;

use super::BookFormat;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "overview.ts")]
pub enum LearningOverviewSource {
    LocalText,
    AiTranscribed,
    AiDescription,
    UserCorrected,
    UserNote,
    HistorySummary,
}

impl LearningOverviewSource {
    pub const ALL: [Self; 6] = [
        Self::LocalText,
        Self::AiTranscribed,
        Self::AiDescription,
        Self::UserCorrected,
        Self::UserNote,
        Self::HistorySummary,
    ];

    pub const fn index(self) -> usize {
        match self {
            Self::LocalText => 0,
            Self::AiTranscribed => 1,
            Self::AiDescription => 2,
            Self::UserCorrected => 3,
            Self::UserNote => 4,
            Self::HistorySummary => 5,
        }
    }

    pub const fn quoteable_as_textbook(self) -> bool {
        matches!(
            self,
            Self::LocalText | Self::AiTranscribed | Self::UserCorrected
        )
    }
}

/// A content-free count for one explicit provenance class.
///
/// `item_count` counts canonical local blocks, effective indexed search items,
/// notes, or completed exchanges according to `source`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "overview.ts")]
pub struct LearningOverviewSourceSummary {
    pub source: LearningOverviewSource,
    pub item_count: u32,
    pub covered_section_count: u32,
    pub covered_page_count: u32,
    pub quoteable_as_textbook: bool,
}

impl LearningOverviewSourceSummary {
    pub const fn empty(source: LearningOverviewSource) -> Self {
        Self {
            source,
            item_count: 0,
            covered_section_count: 0,
            covered_page_count: 0,
            quoteable_as_textbook: source.quoteable_as_textbook(),
        }
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "overview.ts")]
pub struct LearningOverviewSection {
    pub id: Uuid,
    pub parent_id: Option<Uuid>,
    pub ordinal: u32,
    pub title: String,
    pub local_text_item_count: u32,
    pub user_note_count: u32,
    pub completed_conversation_count: u32,
    pub completed_exchange_count: u32,
}

impl fmt::Debug for LearningOverviewSection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LearningOverviewSection")
            .field("id", &self.id)
            .field("parent_id", &self.parent_id)
            .field("ordinal", &self.ordinal)
            .field(
                "title",
                &format_args!("<redacted:{} scalars>", self.title.chars().count()),
            )
            .field("local_text_item_count", &self.local_text_item_count)
            .field("user_note_count", &self.user_note_count)
            .field(
                "completed_conversation_count",
                &self.completed_conversation_count,
            )
            .field("completed_exchange_count", &self.completed_exchange_count)
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "overview.ts")]
pub struct LearningOverviewActivity {
    pub user_note_count: u32,
    pub completed_conversation_count: u32,
    pub completed_exchange_count: u32,
    pub citation_count: u32,
}

#[derive(Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "overview.ts")]
pub struct LearningOverview {
    pub book_id: Uuid,
    pub format: BookFormat,
    pub teaching_instruction_configured: bool,
    pub section_count: u32,
    pub sections: Vec<LearningOverviewSection>,
    pub sources: Vec<LearningOverviewSourceSummary>,
    pub activity: LearningOverviewActivity,
}

impl fmt::Debug for LearningOverview {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LearningOverview")
            .field("book_id", &self.book_id)
            .field("format", &self.format)
            .field(
                "teaching_instruction_configured",
                &self.teaching_instruction_configured,
            )
            .field("section_count", &self.section_count)
            .field("sections", &self.sections)
            .field("sources", &self.sources)
            .field("activity", &self.activity)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[ts(export_to = "overview.ts")]
pub enum LearningOverviewErrorCode {
    #[serde(rename = "LEARNING_OVERVIEW_INVALID_INPUT")]
    InvalidInput,
    #[serde(rename = "LEARNING_OVERVIEW_NOT_FOUND")]
    NotFound,
    #[serde(rename = "LEARNING_OVERVIEW_BOOK_NOT_READY")]
    BookNotReady,
    #[serde(rename = "LEARNING_OVERVIEW_BUSY")]
    Busy,
    #[serde(rename = "LEARNING_OVERVIEW_DATA_INVALID")]
    DataInvalid,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "overview.ts")]
pub struct LearningOverviewErrorDto {
    pub code: LearningOverviewErrorCode,
}

impl LearningOverviewErrorDto {
    pub const fn new(code: LearningOverviewErrorCode) -> Self {
        Self { code }
    }
}
