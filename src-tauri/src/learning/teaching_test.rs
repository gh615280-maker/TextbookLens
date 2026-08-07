use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{
    ai::runtime::ProviderRuntime,
    db::settings,
    domain::{TeachingInstructionDto, UnifiedStreamEvent, validate_teaching_instruction},
    errors::{AppError, AppErrorCode, AppResult},
};

use super::{PromptInput, PromptOperation, PromptPolicy};

pub const MAX_TEACHING_TEST_QUESTION_CODE_POINTS: usize = 500;
pub const MAX_TEACHING_TEST_OUTPUT_CODE_POINTS: usize = 4_000;
const MAX_TEACHING_TEST_INPUT_TOKENS: u64 = 4_096;
const MAX_TEACHING_TEST_OUTPUT_TOKENS: u32 = 512;

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeachingTestRequest {
    pub session_id: Uuid,
    pub request_id: Uuid,
    pub instruction: String,
    pub question: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum TeachingTestEvent {
    TextDelta {
        request_id: Uuid,
        text: String,
    },
    Usage {
        request_id: Uuid,
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
    },
    Completed {
        request_id: Uuid,
    },
    Error {
        request_id: Uuid,
        code: String,
    },
    Cancelled {
        request_id: Uuid,
    },
}

pub async fn run_teaching_test<F>(
    pool: &sqlx::SqlitePool,
    runtime: &ProviderRuntime,
    request: TeachingTestRequest,
    cancel: CancellationToken,
    mut emit: F,
) -> AppResult<()>
where
    F: FnMut(TeachingTestEvent),
{
    validate_request(&request)?;
    if cancel.is_cancelled() {
        return Err(cancelled());
    }

    let settings = settings::get_app_settings(pool).await?;
    let profile_id = settings
        .default_learning_profile_id
        .ok_or_else(|| AppError::new(AppErrorCode::NotFound))?;
    let loaded = runtime
        .load(pool, profile_id, crate::domain::AiOperation::TextLearning)
        .await?;
    if cancel.is_cancelled() {
        return Err(cancelled());
    }

    let prompt = PromptPolicy.prepare(PromptInput {
        operation: PromptOperation::Test,
        expected_language: None,
        book_id: None,
        teaching_instruction: TeachingInstructionDto {
            instruction: request.instruction,
            revision: 0,
            updated_at: chrono::Utc::now(),
        },
        context_segments: Vec::new(),
        prior_messages: Vec::new(),
        current_question: request.question,
        input_budget_tokens: u64::from(loaded.profile().context_window_tokens)
            .min(MAX_TEACHING_TEST_INPUT_TOKENS),
    })?;
    let stream = loaded
        .stream_text_learning(
            prompt.into_chat_request(
                loaded.profile().model_id.clone(),
                MAX_TEACHING_TEST_OUTPUT_TOKENS,
            ),
            cancel.clone(),
        )
        .await?;
    futures_util::pin_mut!(stream);
    let mut output_code_points = 0usize;
    while let Some(event) = stream.next().await {
        if cancel.is_cancelled() {
            return Err(cancelled());
        }
        match event.map_err(|error| error.into_app_error())? {
            UnifiedStreamEvent::TextDelta { text } => {
                output_code_points = output_code_points.saturating_add(text.chars().count());
                if output_code_points > MAX_TEACHING_TEST_OUTPUT_CODE_POINTS {
                    cancel.cancel();
                    return Err(AppError::new(AppErrorCode::ContextTooLarge));
                }
                emit(TeachingTestEvent::TextDelta {
                    request_id: request.request_id,
                    text,
                });
            }
            UnifiedStreamEvent::Usage {
                input_tokens,
                output_tokens,
            } => emit(TeachingTestEvent::Usage {
                request_id: request.request_id,
                input_tokens,
                output_tokens,
            }),
            UnifiedStreamEvent::Completed => {
                if cancel.is_cancelled() {
                    return Err(cancelled());
                }
                emit(TeachingTestEvent::Completed {
                    request_id: request.request_id,
                });
                return Ok(());
            }
        }
    }
    Err(AppError::new(AppErrorCode::ProviderUnavailable))
}

pub fn event_for_result(request_id: Uuid, result: AppResult<()>) -> Option<TeachingTestEvent> {
    match result {
        Ok(()) => None,
        Err(error) if error.code == AppErrorCode::ImportCancelled => {
            Some(TeachingTestEvent::Cancelled { request_id })
        }
        Err(error) => Some(TeachingTestEvent::Error {
            request_id,
            code: error.stable_code().to_owned(),
        }),
    }
}

fn validate_request(request: &TeachingTestRequest) -> AppResult<()> {
    validate_teaching_instruction(&request.instruction)?;
    let question_count = request.question.chars().count();
    if request.question.trim().is_empty()
        || question_count > MAX_TEACHING_TEST_QUESTION_CODE_POINTS
        || request
            .question
            .chars()
            .any(|value| value.is_control() && value != '\n' && value != '\t')
    {
        return Err(AppError::new(AppErrorCode::InvalidInput));
    }
    Ok(())
}

fn cancelled() -> AppError {
    AppError::new(AppErrorCode::ImportCancelled)
}
