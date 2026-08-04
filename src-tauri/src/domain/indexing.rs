use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use super::{IndexCorrectionReviewDto, IndexPageBlockReviewDto};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "indexing.ts")]
pub enum IndexRunStatus {
    Running,
    Paused,
    Cancelling,
    Cancelled,
    Completed,
}

impl IndexRunStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Paused => "paused",
            Self::Cancelling => "cancelling",
            Self::Cancelled => "cancelled",
            Self::Completed => "completed",
        }
    }

    pub fn from_database(value: &str) -> Option<Self> {
        match value {
            "running" => Some(Self::Running),
            "paused" => Some(Self::Paused),
            "cancelling" => Some(Self::Cancelling),
            "cancelled" => Some(Self::Cancelled),
            "completed" => Some(Self::Completed),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "indexing.ts")]
pub enum IndexPageStatus {
    NotRequired,
    Queued,
    Rendering,
    Sending,
    Parsing,
    Validating,
    Indexed,
    NeedsReview,
    Failed,
    Cancelled,
}

impl IndexPageStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NotRequired => "not_required",
            Self::Queued => "queued",
            Self::Rendering => "rendering",
            Self::Sending => "sending",
            Self::Parsing => "parsing",
            Self::Validating => "validating",
            Self::Indexed => "indexed",
            Self::NeedsReview => "needs_review",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn from_database(value: &str) -> Option<Self> {
        match value {
            "not_required" => Some(Self::NotRequired),
            "queued" => Some(Self::Queued),
            "rendering" => Some(Self::Rendering),
            "sending" => Some(Self::Sending),
            "parsing" => Some(Self::Parsing),
            "validating" => Some(Self::Validating),
            "indexed" => Some(Self::Indexed),
            "needs_review" => Some(Self::NeedsReview),
            "failed" => Some(Self::Failed),
            "cancelled" => Some(Self::Cancelled),
            _ => None,
        }
    }

    pub const fn is_in_flight(self) -> bool {
        matches!(
            self,
            Self::Rendering | Self::Sending | Self::Parsing | Self::Validating
        )
    }

    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::NotRequired | Self::Indexed | Self::NeedsReview | Self::Failed | Self::Cancelled
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "indexing.ts")]
pub enum IndexQualityReason {
    ReliableText,
    NoText,
    VeryLowTextCoverage,
    HighReplacementOrControlRatio,
    ExtremeDuplicateGlyphs,
    LayoutContradiction,
}

impl IndexQualityReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ReliableText => "reliable_text",
            Self::NoText => "no_text",
            Self::VeryLowTextCoverage => "very_low_text_coverage",
            Self::HighReplacementOrControlRatio => "high_replacement_or_control_ratio",
            Self::ExtremeDuplicateGlyphs => "extreme_duplicate_glyphs",
            Self::LayoutContradiction => "layout_contradiction",
        }
    }

    pub fn from_database(value: &str) -> Option<Self> {
        match value {
            "reliable_text" => Some(Self::ReliableText),
            "no_text" => Some(Self::NoText),
            "very_low_text_coverage" => Some(Self::VeryLowTextCoverage),
            "high_replacement_or_control_ratio" => Some(Self::HighReplacementOrControlRatio),
            "extreme_duplicate_glyphs" => Some(Self::ExtremeDuplicateGlyphs),
            "layout_contradiction" => Some(Self::LayoutContradiction),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "indexing.ts")]
pub enum IndexAggregateStatus {
    Ready,
    Partial,
    NeedsReview,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[ts(export_to = "indexing.ts")]
pub enum IndexFailureCode {
    IndexRenderFailed,
    IndexProviderFailed,
    IndexResponseInvalid,
    IndexValidationFailed,
    IndexAttemptInterrupted,
    IndexScratchMissing,
    IndexAttemptExpired,
}

impl IndexFailureCode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::IndexRenderFailed => "INDEX_RENDER_FAILED",
            Self::IndexProviderFailed => "INDEX_PROVIDER_FAILED",
            Self::IndexResponseInvalid => "INDEX_RESPONSE_INVALID",
            Self::IndexValidationFailed => "INDEX_VALIDATION_FAILED",
            Self::IndexAttemptInterrupted => "INDEX_ATTEMPT_INTERRUPTED",
            Self::IndexScratchMissing => "INDEX_SCRATCH_MISSING",
            Self::IndexAttemptExpired => "INDEX_ATTEMPT_EXPIRED",
        }
    }

    pub const fn safe_message(self) -> &'static str {
        match self {
            Self::IndexRenderFailed => "The page could not be rendered locally.",
            Self::IndexProviderFailed => "The page analysis request did not complete.",
            Self::IndexResponseInvalid => "The page analysis response was invalid.",
            Self::IndexValidationFailed => "The page analysis did not pass local validation.",
            Self::IndexAttemptInterrupted => "The page attempt was interrupted and can be retried.",
            Self::IndexScratchMissing => {
                "The temporary page render is unavailable; retry the page."
            }
            Self::IndexAttemptExpired => "The interrupted page attempt expired and can be retried.",
        }
    }

    pub fn from_database(value: &str) -> Option<Self> {
        match value {
            "INDEX_RENDER_FAILED" => Some(Self::IndexRenderFailed),
            "INDEX_PROVIDER_FAILED" => Some(Self::IndexProviderFailed),
            "INDEX_RESPONSE_INVALID" => Some(Self::IndexResponseInvalid),
            "INDEX_VALIDATION_FAILED" => Some(Self::IndexValidationFailed),
            "INDEX_ATTEMPT_INTERRUPTED" => Some(Self::IndexAttemptInterrupted),
            "INDEX_SCRATCH_MISSING" => Some(Self::IndexScratchMissing),
            "INDEX_ATTEMPT_EXPIRED" => Some(Self::IndexAttemptExpired),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "indexing.ts")]
pub enum IndexReviewReason {
    IncompleteContent,
    InvalidBounds,
    SevereOverlap,
    TextContradiction,
    MalformedTable,
    MalformedLatex,
}

impl IndexReviewReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::IncompleteContent => "incomplete_content",
            Self::InvalidBounds => "invalid_bounds",
            Self::SevereOverlap => "severe_overlap",
            Self::TextContradiction => "text_contradiction",
            Self::MalformedTable => "malformed_table",
            Self::MalformedLatex => "malformed_latex",
        }
    }

    pub fn from_database(value: &str) -> Option<Self> {
        match value {
            "incomplete_content" => Some(Self::IncompleteContent),
            "invalid_bounds" => Some(Self::InvalidBounds),
            "severe_overlap" => Some(Self::SevereOverlap),
            "text_contradiction" => Some(Self::TextContradiction),
            "malformed_table" => Some(Self::MalformedTable),
            "malformed_latex" => Some(Self::MalformedLatex),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "indexing.ts")]
pub struct SafeIndexErrorDto {
    pub code: IndexFailureCode,
    pub message: String,
    pub retryable: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "indexing.ts")]
pub struct IndexPageStatusCountsDto {
    pub total: u32,
    pub not_required: u32,
    pub queued: u32,
    pub rendering: u32,
    pub sending: u32,
    pub parsing: u32,
    pub validating: u32,
    pub indexed: u32,
    pub needs_review: u32,
    pub failed: u32,
    pub cancelled: u32,
}

impl IndexPageStatusCountsDto {
    pub fn record(&mut self, status: IndexPageStatus) {
        self.total = self.total.saturating_add(1);
        match status {
            IndexPageStatus::NotRequired => self.not_required = self.not_required.saturating_add(1),
            IndexPageStatus::Queued => self.queued = self.queued.saturating_add(1),
            IndexPageStatus::Rendering => self.rendering = self.rendering.saturating_add(1),
            IndexPageStatus::Sending => self.sending = self.sending.saturating_add(1),
            IndexPageStatus::Parsing => self.parsing = self.parsing.saturating_add(1),
            IndexPageStatus::Validating => self.validating = self.validating.saturating_add(1),
            IndexPageStatus::Indexed => self.indexed = self.indexed.saturating_add(1),
            IndexPageStatus::NeedsReview => self.needs_review = self.needs_review.saturating_add(1),
            IndexPageStatus::Failed => self.failed = self.failed.saturating_add(1),
            IndexPageStatus::Cancelled => self.cancelled = self.cancelled.saturating_add(1),
        }
    }

    pub fn aggregate_status(&self) -> IndexAggregateStatus {
        let usable = self.not_required + self.indexed;
        let unresolved =
            self.queued + self.rendering + self.sending + self.parsing + self.validating;
        let unsuccessful = self.failed + self.cancelled;

        if self.total == 0 {
            return IndexAggregateStatus::Failed;
        }
        if unresolved > 0 {
            return IndexAggregateStatus::Partial;
        }
        if unsuccessful > 0 {
            return if usable > 0 || self.needs_review > 0 {
                IndexAggregateStatus::Partial
            } else {
                IndexAggregateStatus::Failed
            };
        }
        if self.needs_review > 0 {
            IndexAggregateStatus::NeedsReview
        } else {
            IndexAggregateStatus::Ready
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "indexing.ts")]
pub struct IndexRunAggregateDto {
    pub run_id: Uuid,
    pub book_id: Uuid,
    pub control_status: IndexRunStatus,
    pub aggregate_status: IndexAggregateStatus,
    pub pages: IndexPageStatusCountsDto,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "indexing.ts")]
pub struct IndexPageReviewDto {
    pub id: Uuid,
    pub run_id: Uuid,
    pub book_id: Uuid,
    pub page_number: u32,
    pub quality_reason: IndexQualityReason,
    pub status: IndexPageStatus,
    pub review_reason: Option<IndexReviewReason>,
    pub safe_error: Option<SafeIndexErrorDto>,
    pub content_version: u32,
    pub blocks: Vec<IndexPageBlockReviewDto>,
    pub corrections: Vec<IndexCorrectionReviewDto>,
    pub updated_at: DateTime<Utc>,
}

pub fn stable_index_page_id(run_id: Uuid, page_number: u32) -> Uuid {
    Uuid::new_v5(&run_id, format!("page:{page_number}").as_bytes())
}

pub fn stable_index_page_block_id(
    book_id: Uuid,
    page_number: u32,
    run_id: Uuid,
    schema_version: &str,
    ordinal: u32,
) -> Uuid {
    Uuid::new_v5(
        &book_id,
        format!("index-block:{page_number}:{run_id}:{schema_version}:{ordinal}").as_bytes(),
    )
}

pub fn stable_index_search_chunk_id(
    block_id: Uuid,
    source: super::ContentSource,
    ordinal: u32,
) -> Uuid {
    Uuid::new_v5(
        &block_id,
        format!("search-chunk:{}:{ordinal}", source.as_str()).as_bytes(),
    )
}
