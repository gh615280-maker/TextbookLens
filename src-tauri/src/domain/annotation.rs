use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use super::SelectionAnchor;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "annotation.ts")]
pub enum AnnotationKind {
    AiConversation,
    Note,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "annotation.ts")]
pub struct AnnotationDto {
    pub id: Uuid,
    pub book_id: Uuid,
    pub section_id: Option<Uuid>,
    pub kind: AnnotationKind,
    pub anchor: Option<SelectionAnchor>,
    pub selected_text: Option<String>,
    pub note_text: Option<String>,
    pub conversation_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
