use std::{fmt, sync::Arc};

use async_trait::async_trait;
use chrono::Utc;
use sqlx::{Row, Sqlite, SqlitePool, Transaction};
use uuid::Uuid;

use crate::{
    domain::{ContentAnchor, LearningAction},
    errors::{AppError, AppErrorCode, AppResult},
};

use super::{
    annotations::{
        delete_ai_conversation_annotation, insert_ai_conversation_annotation,
        validate_anchor_for_write,
    },
    indexing::database_timestamp,
    messages::{
        CompletedAssistantMessage, insert_assistant_message, insert_user_message,
        validate_assistant, validate_question, validated_used_citations_json,
    },
};

const MAX_ANCHOR_JSON_BYTES: usize = 128 * 1024;
const MAX_SELECTED_TEXT_CODE_POINTS: usize = 1_048_576;
const MAX_SELECTED_TEXT_BYTES: usize = 2 * 1024 * 1024;

#[derive(Clone)]
pub struct NewSelectionCompletion {
    pub book_id: Uuid,
    pub section_id: Uuid,
    pub anchor: ContentAnchor,
    pub selected_text: Option<String>,
    pub action: LearningAction,
    pub question: String,
    pub assistant: CompletedAssistantMessage,
}

impl fmt::Debug for NewSelectionCompletion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NewSelectionCompletion")
            .field("book_id", &"<redacted>")
            .field("section_id", &"<redacted>")
            .field("anchor", &"<redacted>")
            .field(
                "selected_text_bytes",
                &self.selected_text.as_ref().map(String::len),
            )
            .field("action", &self.action)
            .field("question_bytes", &self.question.len())
            .field("assistant", &self.assistant)
            .finish()
    }
}

#[derive(Clone)]
pub struct FollowupCompletion {
    pub book_id: Uuid,
    pub conversation_id: Uuid,
    pub expected_next_ordinal: u32,
    pub question: String,
    pub assistant: CompletedAssistantMessage,
}

impl fmt::Debug for FollowupCompletion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FollowupCompletion")
            .field("book_id", &"<redacted>")
            .field("conversation_id", &"<redacted>")
            .field("expected_next_ordinal", &self.expected_next_ordinal)
            .field("question_bytes", &self.question.len())
            .field("assistant", &self.assistant)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PersistedLearningResult {
    pub conversation_id: Uuid,
    pub annotation_id: Uuid,
}

#[derive(Clone, Copy, Debug)]
pub struct DeleteSelectionConversation {
    pub book_id: Uuid,
    pub conversation_id: Uuid,
    pub annotation_id: Uuid,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LearningPersistenceStep {
    ConversationLock,
    ConversationInsert,
    UserMessageInsert,
    AssistantMessageInsert,
    AnnotationInsert,
    ConversationUpdate,
    AnnotationDelete,
    Commit,
}

#[async_trait]
pub trait LearningPersistenceFaultInjector: Send + Sync {
    async fn checkpoint(&self, step: LearningPersistenceStep) -> AppResult<()>;
}

struct NoLearningPersistenceFaults;

#[async_trait]
impl LearningPersistenceFaultInjector for NoLearningPersistenceFaults {
    async fn checkpoint(&self, _step: LearningPersistenceStep) -> AppResult<()> {
        Ok(())
    }
}

#[derive(Clone)]
pub struct LearningRepository {
    pool: SqlitePool,
    fault_injector: Arc<dyn LearningPersistenceFaultInjector>,
}

impl LearningRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self {
            pool,
            fault_injector: Arc::new(NoLearningPersistenceFaults),
        }
    }

    #[doc(hidden)]
    pub fn with_fault_injector(
        pool: SqlitePool,
        fault_injector: Arc<dyn LearningPersistenceFaultInjector>,
    ) -> Self {
        Self {
            pool,
            fault_injector,
        }
    }

    pub async fn persist_new_selection(
        &self,
        input: NewSelectionCompletion,
    ) -> AppResult<PersistedLearningResult> {
        validate_new_selection(&input)?;
        validate_anchor_for_write(&self.pool, input.book_id, input.section_id, &input.anchor)
            .await?;
        let anchor_json = serde_json::to_string(&input.anchor).map_err(|_| database_error())?;
        if anchor_json.len() > MAX_ANCHOR_JSON_BYTES {
            return Err(invalid_input());
        }

        let conversation_id = Uuid::new_v4();
        let annotation_id = Uuid::new_v4();
        let user_message_id = Uuid::new_v4();
        let assistant_message_id = Uuid::new_v4();
        let timestamp = database_timestamp(Utc::now());
        let anchor_kind = match &input.anchor {
            ContentAnchor::Text { .. } => "text",
            ContentAnchor::Region { .. } => "region",
        };
        let mut transaction = self.pool.begin().await?;
        require_ready_book_and_section(&mut transaction, input.book_id, input.section_id).await?;

        self.checkpoint(LearningPersistenceStep::ConversationInsert)
            .await?;
        let inserted = sqlx::query(
            "INSERT INTO conversations (id, book_id, section_id, scope, anchor_kind, anchor_json, selected_text, created_at, updated_at) VALUES (?, ?, ?, 'selection', ?, ?, ?, ?, ?)",
        )
        .bind(conversation_id.to_string())
        .bind(input.book_id.to_string())
        .bind(input.section_id.to_string())
        .bind(anchor_kind)
        .bind(&anchor_json)
        .bind(input.selected_text.as_deref())
        .bind(&timestamp)
        .bind(&timestamp)
        .execute(&mut *transaction)
        .await?;
        if inserted.rows_affected() != 1 {
            return Err(database_error());
        }

        let citations_json = validated_used_citations_json(
            &mut transaction,
            input.book_id,
            &input.assistant.answer,
            &input.assistant.available_citations,
        )
        .await?;

        self.checkpoint(LearningPersistenceStep::UserMessageInsert)
            .await?;
        insert_user_message(
            &mut transaction,
            user_message_id,
            conversation_id,
            0,
            input.action.clone(),
            &input.question,
            &timestamp,
        )
        .await?;

        self.checkpoint(LearningPersistenceStep::AssistantMessageInsert)
            .await?;
        insert_assistant_message(
            &mut transaction,
            assistant_message_id,
            conversation_id,
            1,
            input.action,
            &input.assistant,
            &citations_json,
            &timestamp,
        )
        .await?;

        self.checkpoint(LearningPersistenceStep::AnnotationInsert)
            .await?;
        insert_ai_conversation_annotation(
            &mut transaction,
            annotation_id,
            input.book_id,
            input.section_id,
            &anchor_json,
            input.selected_text.as_deref(),
            conversation_id,
            &timestamp,
        )
        .await?;

        self.checkpoint(LearningPersistenceStep::Commit).await?;
        transaction.commit().await?;
        Ok(PersistedLearningResult {
            conversation_id,
            annotation_id,
        })
    }

    pub async fn persist_followup(&self, input: FollowupCompletion) -> AppResult<()> {
        validate_question(&input.question)?;
        validate_assistant(&input.assistant)?;
        if input.expected_next_ordinal < 2 || !input.expected_next_ordinal.is_multiple_of(2) {
            return Err(invalid_input());
        }
        let timestamp = database_timestamp(Utc::now());
        let mut transaction = self.pool.begin().await?;

        self.checkpoint(LearningPersistenceStep::ConversationLock)
            .await?;
        let locked = sqlx::query(
            "UPDATE conversations SET updated_at = updated_at WHERE id = ? AND book_id = ? AND scope = 'selection'",
        )
        .bind(input.conversation_id.to_string())
        .bind(input.book_id.to_string())
        .execute(&mut *transaction)
        .await?;
        if locked.rows_affected() != 1 {
            return Err(AppError::new(AppErrorCode::NotFound));
        }
        require_selection_annotation(&mut transaction, input.book_id, input.conversation_id)
            .await?;
        let next_ordinal = next_message_ordinal(&mut transaction, input.conversation_id).await?;
        if next_ordinal != input.expected_next_ordinal {
            return Err(AppError::new(AppErrorCode::RequestConflict));
        }
        let assistant_ordinal = next_ordinal.checked_add(1).ok_or_else(invalid_input)?;
        let citations_json = validated_used_citations_json(
            &mut transaction,
            input.book_id,
            &input.assistant.answer,
            &input.assistant.available_citations,
        )
        .await?;

        self.checkpoint(LearningPersistenceStep::UserMessageInsert)
            .await?;
        insert_user_message(
            &mut transaction,
            Uuid::new_v4(),
            input.conversation_id,
            next_ordinal,
            LearningAction::Continue,
            &input.question,
            &timestamp,
        )
        .await?;

        self.checkpoint(LearningPersistenceStep::AssistantMessageInsert)
            .await?;
        insert_assistant_message(
            &mut transaction,
            Uuid::new_v4(),
            input.conversation_id,
            assistant_ordinal,
            LearningAction::Continue,
            &input.assistant,
            &citations_json,
            &timestamp,
        )
        .await?;

        self.checkpoint(LearningPersistenceStep::ConversationUpdate)
            .await?;
        let updated = sqlx::query(
            "UPDATE conversations SET updated_at = ? WHERE id = ? AND book_id = ? AND scope = 'selection'",
        )
        .bind(&timestamp)
        .bind(input.conversation_id.to_string())
        .bind(input.book_id.to_string())
        .execute(&mut *transaction)
        .await?;
        if updated.rows_affected() != 1 {
            return Err(database_error());
        }

        self.checkpoint(LearningPersistenceStep::Commit).await?;
        transaction.commit().await?;
        Ok(())
    }

    pub async fn delete_selection_conversation(
        &self,
        input: DeleteSelectionConversation,
    ) -> AppResult<()> {
        let mut transaction = self.pool.begin().await?;
        self.checkpoint(LearningPersistenceStep::ConversationLock)
            .await?;
        let locked = sqlx::query(
            "UPDATE conversations SET updated_at = updated_at WHERE id = ? AND book_id = ? AND scope = 'selection'",
        )
        .bind(input.conversation_id.to_string())
        .bind(input.book_id.to_string())
        .execute(&mut *transaction)
        .await?;
        if locked.rows_affected() != 1 {
            return Err(AppError::new(AppErrorCode::NotFound));
        }
        let annotation_exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM annotations WHERE id = ? AND book_id = ? AND kind = 'ai_conversation' AND conversation_id = ?)",
        )
        .bind(input.annotation_id.to_string())
        .bind(input.book_id.to_string())
        .bind(input.conversation_id.to_string())
        .fetch_one(&mut *transaction)
        .await?;
        if !annotation_exists {
            return Err(AppError::new(AppErrorCode::NotFound));
        }

        self.checkpoint(LearningPersistenceStep::AnnotationDelete)
            .await?;
        if delete_ai_conversation_annotation(
            &mut transaction,
            input.annotation_id,
            input.book_id,
            input.conversation_id,
        )
        .await?
            != 1
        {
            return Err(AppError::new(AppErrorCode::RequestConflict));
        }
        let remaining_conversation: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM conversations WHERE id = ?) OR EXISTS(SELECT 1 FROM messages WHERE conversation_id = ?)",
        )
        .bind(input.conversation_id.to_string())
        .bind(input.conversation_id.to_string())
        .fetch_one(&mut *transaction)
        .await?;
        if remaining_conversation {
            return Err(database_error());
        }

        self.checkpoint(LearningPersistenceStep::Commit).await?;
        transaction.commit().await?;
        Ok(())
    }

    async fn checkpoint(&self, step: LearningPersistenceStep) -> AppResult<()> {
        self.fault_injector.checkpoint(step).await
    }
}

impl fmt::Debug for LearningRepository {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LearningRepository")
            .finish_non_exhaustive()
    }
}

fn validate_new_selection(input: &NewSelectionCompletion) -> AppResult<()> {
    validate_question(&input.question)?;
    validate_assistant(&input.assistant)?;
    if matches!(
        input.action,
        LearningAction::Continue | LearningAction::Overview
    ) {
        return Err(invalid_input());
    }
    let expected_selected_text = match &input.anchor {
        ContentAnchor::Text { selection } => Some(selection.quote.exact.as_str()),
        ContentAnchor::Region { region } => region
            .text_fallback
            .as_ref()
            .map(|fallback| fallback.exact.as_str()),
    };
    if input.selected_text.as_deref() != expected_selected_text
        || input.selected_text.as_ref().is_some_and(|text| {
            text.trim().is_empty()
                || text.len() > MAX_SELECTED_TEXT_BYTES
                || text.chars().count() > MAX_SELECTED_TEXT_CODE_POINTS
                || contains_disallowed_control(text)
        })
    {
        return Err(invalid_input());
    }
    Ok(())
}

async fn require_ready_book_and_section(
    transaction: &mut Transaction<'_, Sqlite>,
    book_id: Uuid,
    section_id: Uuid,
) -> AppResult<()> {
    let status = sqlx::query_scalar::<_, String>("SELECT import_status FROM books WHERE id = ?")
        .bind(book_id.to_string())
        .fetch_optional(&mut **transaction)
        .await?
        .ok_or_else(|| AppError::new(AppErrorCode::NotFound))?;
    if status != "ready" {
        return Err(AppError::new(AppErrorCode::BookNotReady));
    }
    let section_exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM sections WHERE id = ? AND book_id = ?)")
            .bind(section_id.to_string())
            .bind(book_id.to_string())
            .fetch_one(&mut **transaction)
            .await?;
    if !section_exists {
        return Err(AppError::new(AppErrorCode::AnchorNotFound));
    }
    Ok(())
}

async fn require_selection_annotation(
    transaction: &mut Transaction<'_, Sqlite>,
    book_id: Uuid,
    conversation_id: Uuid,
) -> AppResult<()> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM annotations WHERE book_id = ? AND kind = 'ai_conversation' AND conversation_id = ?",
    )
    .bind(book_id.to_string())
    .bind(conversation_id.to_string())
    .fetch_one(&mut **transaction)
    .await?;
    if count != 1 {
        return Err(database_error());
    }
    Ok(())
}

async fn next_message_ordinal(
    transaction: &mut Transaction<'_, Sqlite>,
    conversation_id: Uuid,
) -> AppResult<u32> {
    let row = sqlx::query(
        "SELECT COUNT(*) AS message_count, MAX(ordinal) AS max_ordinal FROM messages WHERE conversation_id = ?",
    )
    .bind(conversation_id.to_string())
    .fetch_one(&mut **transaction)
    .await?;
    let count: i64 = row.try_get("message_count")?;
    let max_ordinal: Option<i64> = row.try_get("max_ordinal")?;
    if count < 2 || max_ordinal != Some(count - 1) || count % 2 != 0 {
        return Err(database_error());
    }
    u32::try_from(count).map_err(|_| database_error())
}

fn contains_disallowed_control(value: &str) -> bool {
    value
        .chars()
        .any(|code_point| code_point.is_control() && code_point != '\n' && code_point != '\t')
}

fn invalid_input() -> AppError {
    AppError::new(AppErrorCode::InvalidInput)
}

fn database_error() -> AppError {
    AppError::new(AppErrorCode::DatabaseError)
}

#[cfg(test)]
#[path = "conversations_test.rs"]
mod tests;
