use serde::Serialize;
use tauri::State;
use uuid::Uuid;

use crate::{
    app_state::AppState,
    db::conversations::{
        BookConversationHistoryDto, ConversationHistoryDto, DeleteBookConversation,
        DeleteSelectionConversation, LearningRepository, load_book_conversation,
        load_selection_conversation,
    },
    errors::AppError,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ConversationErrorDto {
    code: &'static str,
}

impl From<AppError> for ConversationErrorDto {
    fn from(error: AppError) -> Self {
        Self {
            code: error.stable_code(),
        }
    }
}

#[tauri::command]
pub async fn get_learning_conversation(
    state: State<'_, AppState>,
    book_id: Uuid,
    conversation_id: Uuid,
) -> Result<ConversationHistoryDto, ConversationErrorDto> {
    load_selection_conversation(state.db.pool(), book_id, conversation_id)
        .await
        .map_err(ConversationErrorDto::from)
}

#[tauri::command]
pub async fn delete_learning_conversation(
    state: State<'_, AppState>,
    book_id: Uuid,
    conversation_id: Uuid,
    annotation_id: Uuid,
) -> Result<(), ConversationErrorDto> {
    let _deletion_guard = state
        .learning_requests
        .begin_conversation_deletion(conversation_id)
        .map_err(ConversationErrorDto::from)?;
    LearningRepository::new(state.db.pool().clone())
        .delete_selection_conversation(DeleteSelectionConversation {
            book_id,
            conversation_id,
            annotation_id,
        })
        .await
        .map_err(ConversationErrorDto::from)
}

#[tauri::command]
pub async fn get_book_learning_conversation(
    state: State<'_, AppState>,
    book_id: Uuid,
    conversation_id: Uuid,
) -> Result<BookConversationHistoryDto, ConversationErrorDto> {
    load_book_conversation(state.db.pool(), book_id, conversation_id)
        .await
        .map_err(ConversationErrorDto::from)
}

#[tauri::command]
pub async fn delete_book_learning_conversation(
    state: State<'_, AppState>,
    book_id: Uuid,
    conversation_id: Uuid,
) -> Result<(), ConversationErrorDto> {
    let _deletion_guard = state
        .learning_requests
        .begin_conversation_deletion(conversation_id)
        .map_err(ConversationErrorDto::from)?;
    LearningRepository::new(state.db.pool().clone())
        .delete_book_conversation(DeleteBookConversation {
            book_id,
            conversation_id,
        })
        .await
        .map_err(ConversationErrorDto::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::{AppError, AppErrorCode};

    #[test]
    fn conversation_error_dto_contains_only_the_stable_code() {
        let serialized = serde_json::to_value(ConversationErrorDto::from(AppError::new(
            AppErrorCode::RequestConflict,
        )))
        .unwrap();
        assert_eq!(
            serialized,
            serde_json::json!({ "code": "REQUEST_CONFLICT" })
        );
    }
}
