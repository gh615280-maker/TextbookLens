use std::fmt;

use serde::Serialize;
use uuid::Uuid;

use crate::domain::{Citation, CitationReviewStatus, ContentSource};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextSource {
    LocalText,
    AiTranscribed,
    AiDescription,
    UserCorrected,
    UserNote,
    HistorySummary,
    /// Structural table-of-contents metadata, never verbatim textbook evidence.
    Directory,
}

impl ContextSource {
    pub const fn citation_policy(self) -> &'static str {
        match self {
            Self::LocalText => "textbook_source",
            Self::AiTranscribed => "textbook_source_label_ai_transcription",
            Self::AiDescription => "auxiliary_not_verbatim_textbook_source",
            Self::UserCorrected => "textbook_source_label_user_correction",
            Self::UserNote => "user_note_not_textbook_source",
            Self::HistorySummary => "history_summary_not_textbook_source",
            Self::Directory => "table_of_contents_metadata_not_textbook_source",
        }
    }

    pub const fn quoteable(self) -> bool {
        matches!(
            self,
            Self::LocalText | Self::AiTranscribed | Self::UserCorrected
        )
    }

    pub const fn content_source(self) -> Option<ContentSource> {
        match self {
            Self::LocalText => Some(ContentSource::LocalText),
            Self::AiTranscribed => Some(ContentSource::AiTranscribed),
            Self::AiDescription => Some(ContentSource::AiDescription),
            Self::UserCorrected => Some(ContentSource::UserCorrected),
            Self::UserNote | Self::HistorySummary | Self::Directory => None,
        }
    }
}

impl From<ContentSource> for ContextSource {
    fn from(source: ContentSource) -> Self {
        match source {
            ContentSource::LocalText => Self::LocalText,
            ContentSource::AiTranscribed => Self::AiTranscribed,
            ContentSource::AiDescription => Self::AiDescription,
            ContentSource::UserCorrected => Self::UserCorrected,
        }
    }
}

#[derive(Clone, PartialEq)]
pub struct TypedContextSegment {
    pub book_id: Uuid,
    pub source: ContextSource,
    pub locator: String,
    pub review_status: CitationReviewStatus,
    pub citation: Option<Citation>,
    pub content: String,
}

impl fmt::Debug for TypedContextSegment {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TypedContextSegment")
            .field("book_id", &"<redacted>")
            .field("source", &self.source)
            .field("locator", &self.locator)
            .field("review_status", &self.review_status)
            .field("citation", &self.citation)
            .field(
                "content",
                &format_args!("<redacted:{} scalars>", self.content.chars().count()),
            )
            .finish()
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PromptContextSegment<'a> {
    source: ContextSource,
    locator: &'a str,
    citation_policy: &'static str,
    review_status: CitationReviewStatus,
    quoteable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    citation_id: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    citation_label: Option<&'a str>,
    content: &'a str,
}

impl<'a> From<&'a TypedContextSegment> for PromptContextSegment<'a> {
    fn from(segment: &'a TypedContextSegment) -> Self {
        Self {
            source: segment.source,
            locator: &segment.locator,
            citation_policy: segment.source.citation_policy(),
            review_status: segment.review_status,
            quoteable: segment.source.quoteable(),
            citation_id: segment
                .citation
                .as_ref()
                .map(|citation| citation.id.as_str()),
            citation_label: segment
                .citation
                .as_ref()
                .map(|citation| citation.label.as_str()),
            content: &segment.content,
        }
    }
}
