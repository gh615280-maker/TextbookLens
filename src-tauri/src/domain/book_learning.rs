use std::fmt;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;
use zeroize::Zeroize;

/// The only frontend-controlled fields accepted while preparing a textbook-level question.
///
/// Provider bindings, teaching preferences, the table of contents, retrieval results, history,
/// prompts, and citations are deliberately absent. Rust resolves all of them from local
/// authoritative state.
#[derive(Clone, PartialEq, Eq, Deserialize, TS)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
#[ts(export_to = "book_learning.ts")]
pub enum PrepareBookLearningRequestMetadata {
    New {
        book_id: Uuid,
        question: String,
    },
    Continue {
        book_id: Uuid,
        conversation_id: Uuid,
        question: String,
    },
}

impl PrepareBookLearningRequestMetadata {
    pub const fn book_id(&self) -> Uuid {
        match self {
            Self::New { book_id, .. } | Self::Continue { book_id, .. } => *book_id,
        }
    }

    pub fn question(&self) -> &str {
        match self {
            Self::New { question, .. } | Self::Continue { question, .. } => question,
        }
    }

    pub const fn conversation_id(&self) -> Option<Uuid> {
        match self {
            Self::New { .. } => None,
            Self::Continue {
                conversation_id, ..
            } => Some(*conversation_id),
        }
    }
}

impl fmt::Debug for PrepareBookLearningRequestMetadata {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PrepareBookLearningRequestMetadata")
            .field(
                "kind",
                &match self {
                    Self::New { .. } => "new",
                    Self::Continue { .. } => "continue",
                },
            )
            .field("book_id", &"<redacted>")
            .field(
                "conversation_id",
                &self.conversation_id().map(|_| "<redacted>"),
            )
            .field("question_code_points", &self.question().chars().count())
            .finish()
    }
}

impl Drop for PrepareBookLearningRequestMetadata {
    fn drop(&mut self) {
        match self {
            Self::New { question, .. } | Self::Continue { question, .. } => question.zeroize(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "book_learning.ts")]
pub enum BookLearningPreparationRiskFlag {
    CostRisk,
}

/// Content-free preparation metadata safe to return over IPC.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "book_learning.ts")]
pub struct BookLearningPreparationSummary {
    pub preparation_id: Uuid,
    pub provider_display_name: String,
    pub profile_display_name: String,
    pub model_display_name: String,
    pub estimated_input_tokens: u32,
    pub source_count: u32,
    pub citation_count: u32,
    pub omitted_source_count: u32,
    pub risk_flags: Vec<BookLearningPreparationRiskFlag>,
    pub requires_blocking_confirmation: bool,
    pub expires_at: DateTime<Utc>,
}
