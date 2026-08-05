use std::fmt;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use super::{ContentAnchor, ContentSource, DocumentLocator};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "conversation.ts")]
pub enum ConversationScope {
    Selection,
    Book,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "conversation.ts")]
pub enum ConversationAnchorKind {
    Text,
    Region,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "conversation.ts")]
pub enum LearningAction {
    Explain,
    Example,
    Derive,
    Translate,
    Ask,
    Continue,
    Overview,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "conversation.ts")]
pub enum CitationReviewStatus {
    #[default]
    NotRequired,
    Indexed,
    NeedsReview,
    UserCorrected,
}

#[derive(Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "conversation.ts")]
pub struct Citation {
    pub id: String,
    pub label: String,
    pub book_id: Uuid,
    pub section_id: Option<Uuid>,
    pub locator: DocumentLocator,
    pub source: ContentSource,
    pub review_status: CitationReviewStatus,
    pub quoteable: bool,
}

impl Citation {
    pub fn new(
        id: String,
        label: String,
        book_id: Uuid,
        section_id: Option<Uuid>,
        locator: DocumentLocator,
        source: ContentSource,
        review_status: CitationReviewStatus,
    ) -> Result<Self, &'static str> {
        if !is_request_local_citation_id(&id) {
            return Err("citation ID must be a positive request-local TL-C identifier");
        }
        if label.trim().is_empty()
            || label.chars().count() > 256
            || label
                .chars()
                .any(|value| value.is_control() && value != '\t')
        {
            return Err("citation label is invalid");
        }
        if source == ContentSource::AiDescription {
            return Err("AI descriptions are not quoteable textbook sources");
        }
        let review_status_matches_source = match source {
            ContentSource::LocalText => review_status == CitationReviewStatus::NotRequired,
            ContentSource::AiTranscribed => matches!(
                review_status,
                CitationReviewStatus::Indexed | CitationReviewStatus::NeedsReview
            ),
            ContentSource::UserCorrected => review_status == CitationReviewStatus::UserCorrected,
            ContentSource::AiDescription => false,
        };
        if !review_status_matches_source {
            return Err("citation review status does not match its source");
        }
        Ok(Self {
            id,
            label,
            book_id,
            section_id,
            locator,
            source,
            review_status,
            quoteable: true,
        })
    }
}

impl fmt::Debug for Citation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Citation")
            .field("id", &self.id)
            .field("label", &self.label)
            .field("source", &self.source)
            .field("review_status", &self.review_status)
            .field("quoteable", &self.quoteable)
            .field("book_id", &"<redacted>")
            .field("section_id", &self.section_id.map(|_| "<redacted>"))
            .field("locator", &"<redacted>")
            .finish()
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SerializedCitation {
    id: String,
    label: String,
    book_id: Uuid,
    section_id: Option<Uuid>,
    locator: DocumentLocator,
    source: ContentSource,
    review_status: CitationReviewStatus,
    quoteable: bool,
}

impl<'de> Deserialize<'de> for Citation {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = SerializedCitation::deserialize(deserializer)?;
        if !value.quoteable {
            return Err(serde::de::Error::custom(
                "persisted citations must be quoteable",
            ));
        }
        Self::new(
            value.id,
            value.label,
            value.book_id,
            value.section_id,
            value.locator,
            value.source,
            value.review_status,
        )
        .map_err(serde::de::Error::custom)
    }
}

fn is_request_local_citation_id(value: &str) -> bool {
    value.strip_prefix("TL-C").is_some_and(|suffix| {
        !suffix.starts_with('0')
            && !suffix.is_empty()
            && suffix.bytes().all(|value| value.is_ascii_digit())
            && suffix.parse::<u32>().is_ok()
    })
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "conversation.ts")]
pub struct ConversationDto {
    pub id: Uuid,
    pub book_id: Uuid,
    pub section_id: Option<Uuid>,
    pub scope: ConversationScope,
    pub anchor_kind: Option<ConversationAnchorKind>,
    pub anchor: Option<ContentAnchor>,
    pub selected_text: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "scope", rename_all = "snake_case")]
#[ts(export_to = "conversation.ts")]
pub enum LearningRequest {
    NewSelection {
        request_id: Uuid,
        book_id: Uuid,
        section_id: Uuid,
        anchor: Box<ContentAnchor>,
        selected_text: String,
        action: LearningAction,
        question: Option<String>,
        target_language: Option<String>,
    },
    ContinueSelection {
        request_id: Uuid,
        book_id: Uuid,
        conversation_id: Uuid,
        question: String,
    },
    NewBookQuestion {
        request_id: Uuid,
        book_id: Uuid,
        question: String,
    },
    ContinueBookQuestion {
        request_id: Uuid,
        book_id: Uuid,
        conversation_id: Uuid,
        question: String,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export_to = "conversation.ts")]
pub enum LearningEvent {
    Preparing,
    TextDelta {
        text: String,
    },
    Usage {
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
    },
    Persisted {
        conversation_id: Uuid,
        annotation_id: Option<Uuid>,
    },
    Failed {
        code: String,
    },
    Cancelled,
}
