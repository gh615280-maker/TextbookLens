use serde::Serialize;
use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextSource {
    LocalText,
    AiTranscribed,
    AiDescription,
    UserCorrected,
    UserNote,
    HistorySummary,
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
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypedContextSegment {
    pub book_id: Uuid,
    pub source: ContextSource,
    pub locator: String,
    pub content: String,
}

#[derive(Serialize)]
pub(crate) struct PromptContextSegment<'a> {
    source: ContextSource,
    locator: &'a str,
    citation_policy: &'static str,
    content: &'a str,
}

impl<'a> From<&'a TypedContextSegment> for PromptContextSegment<'a> {
    fn from(segment: &'a TypedContextSegment) -> Self {
        Self {
            source: segment.source,
            locator: &segment.locator,
            citation_policy: segment.source.citation_policy(),
            content: &segment.content,
        }
    }
}
