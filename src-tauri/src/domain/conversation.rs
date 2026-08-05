use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use super::ContentAnchor;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "conversation.ts")]
pub enum ConversationScope {
    Selection,
    Book,
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "conversation.ts")]
pub struct ConversationDto {
    pub id: Uuid,
    pub book_id: Uuid,
    pub section_id: Option<Uuid>,
    pub scope: ConversationScope,
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
