use std::{fmt, sync::Arc};

use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::{
    ai::registry::ProviderCapabilityRegistry,
    db::{messages::validate_question, providers, settings, teaching},
    domain::{AiOperation, ContentAnchor, UnifiedChatRequest, UnifiedMessage, UnifiedRole},
    errors::{AppError, AppErrorCode, AppResult},
    retrieval::{
        budget::{InputBudget, conservative_token_count},
        context::{ContextCandidate, SelectionContextQuery, retrieve_selection_context},
    },
};

use super::{
    PromptInput, PromptOperation, PromptPolicy,
    registry::{FollowupRequestContext, LearningPersistenceTarget, LearningRequestContext},
};

const MAX_HISTORY_MESSAGES_LOADED: i64 = 64;
const MESSAGE_FRAMING_TOKENS: u64 = 16;
const REGION_WITHOUT_TEXT_PLACEHOLDER: &str =
    "[Visual region from this textbook; no image bytes were retained for this follow-up.]";

pub struct PreparedFollowupExecution {
    pub operation: AiOperation,
    pub chat_request: UnifiedChatRequest,
    pub context: Arc<LearningRequestContext>,
}

impl fmt::Debug for PreparedFollowupExecution {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PreparedFollowupExecution")
            .field("operation", &self.operation)
            .field("chat_request", &"<redacted>")
            .field("context", &self.context)
            .finish()
    }
}

pub async fn prepare_conversation_followup(
    pool: &SqlitePool,
    capabilities: &ProviderCapabilityRegistry,
    conversation_id: Uuid,
    question: String,
) -> AppResult<PreparedFollowupExecution> {
    validate_question(&question)?;
    let conversation = sqlx::query(
        "SELECT book_id, section_id, scope, anchor_json, selected_text FROM conversations WHERE id = ?",
    )
    .bind(conversation_id.to_string())
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::new(AppErrorCode::NotFound))?;
    if conversation.try_get::<String, _>("scope")? != "selection" {
        return Err(AppError::new(AppErrorCode::InvalidInput));
    }
    let book_id = parse_uuid(conversation.try_get("book_id")?)?;
    let section_id = conversation
        .try_get::<Option<String>, _>("section_id")?
        .map(parse_uuid)
        .transpose()?
        .ok_or_else(|| AppError::new(AppErrorCode::DatabaseError))?;
    let anchor: ContentAnchor =
        serde_json::from_str(&conversation.try_get::<String, _>("anchor_json")?)
            .map_err(|_| AppError::new(AppErrorCode::DatabaseError))?;
    let selected_text = conversation
        .try_get::<Option<String>, _>("selected_text")?
        .unwrap_or_else(|| REGION_WITHOUT_TEXT_PLACEHOLDER.to_owned());

    let (expected_next_ordinal, history) = load_bounded_history(pool, conversation_id).await?;
    let app_settings = settings::get_app_settings(pool).await?;
    let profile_id = app_settings
        .default_learning_profile_id
        .ok_or_else(|| AppError::new(AppErrorCode::NotFound))?;
    let profile = providers::load_provider_profile_metadata(pool, profile_id).await?;
    let model = capabilities
        .capabilities()
        .iter()
        .find(|provider| provider.kind == profile.kind)
        .and_then(|provider| {
            provider
                .models
                .iter()
                .find(|model| model.id == profile.model_id)
        })
        .ok_or_else(AppError::unsupported_provider_capability)?;
    if capabilities.operation_support(&profile.kind, &profile.model_id, AiOperation::TextLearning)
        != crate::domain::CapabilitySupport::Supported
    {
        return Err(AppError::unsupported_provider_capability());
    }
    let model_id = profile.model_id.clone();
    let max_output_tokens = model.default_max_output_tokens;
    let budget = InputBudget::new(
        profile
            .context_window_tokens
            .min(model.context_window_tokens),
        app_settings.context_mode,
        max_output_tokens,
    );

    let mut candidates = retrieve_selection_context(
        pool,
        &SelectionContextQuery {
            book_id,
            section_id,
            anchor: anchor.clone(),
            selected_text,
            query_text: question.clone(),
        },
    )
    .await?;
    if expected_next_ordinal > 0 {
        candidates.push(ContextCandidate::history_placeholder(
            book_id,
            Some(section_id),
        ));
    }
    let instruction = teaching::get_teaching_instruction(pool).await?;
    let (mut prompt, packed) = PromptPolicy.pack_and_prepare(
        PromptInput {
            operation: PromptOperation::Continue,
            book_id: Some(book_id),
            teaching_instruction: instruction,
            context_segments: Vec::new(),
            prior_messages: Vec::new(),
            current_question: question.clone(),
            input_budget_tokens: u64::from(budget.usable_input),
        },
        budget,
        candidates,
    )?;
    let current = prompt
        .messages
        .pop()
        .ok_or_else(|| AppError::new(AppErrorCode::DatabaseError))?;
    let history_budget =
        u64::from(budget.usable_input).saturating_sub(prompt.cost.conservative_tokens);
    let mut retained = retain_recent_history_pairs(history, history_budget);
    retained.push(current);
    prompt.messages = retained;

    Ok(PreparedFollowupExecution {
        operation: AiOperation::TextLearning,
        chat_request: prompt.into_chat_request(model_id.clone(), max_output_tokens, None),
        context: Arc::new(LearningRequestContext {
            target: LearningPersistenceTarget::Followup(FollowupRequestContext {
                book_id,
                conversation_id,
                expected_next_ordinal,
                question,
            }),
            provider_profile_id: profile_id,
            model_id,
            available_citations: packed.citations,
        }),
    })
}

async fn load_bounded_history(
    pool: &SqlitePool,
    conversation_id: Uuid,
) -> AppResult<(u32, Vec<UnifiedMessage>)> {
    let aggregate = sqlx::query(
        "SELECT COUNT(*) AS message_count, MAX(ordinal) AS max_ordinal FROM messages WHERE conversation_id = ?",
    )
    .bind(conversation_id.to_string())
    .fetch_one(pool)
    .await?;
    let count: i64 = aggregate.try_get("message_count")?;
    let max_ordinal: Option<i64> = aggregate.try_get("max_ordinal")?;
    if count < 2 || count % 2 != 0 || max_ordinal != Some(count - 1) {
        return Err(AppError::new(AppErrorCode::DatabaseError));
    }
    let expected_next_ordinal =
        u32::try_from(count).map_err(|_| AppError::new(AppErrorCode::ContextTooLarge))?;
    let mut rows = sqlx::query(
        "SELECT ordinal, role, content FROM messages WHERE conversation_id = ? ORDER BY ordinal DESC LIMIT ?",
    )
    .bind(conversation_id.to_string())
    .bind(MAX_HISTORY_MESSAGES_LOADED)
    .fetch_all(pool)
    .await?;
    rows.reverse();
    if rows.len() % 2 != 0 {
        rows.remove(0);
    }
    let first_expected = count
        .checked_sub(
            i64::try_from(rows.len()).map_err(|_| AppError::new(AppErrorCode::DatabaseError))?,
        )
        .ok_or_else(|| AppError::new(AppErrorCode::DatabaseError))?;
    let mut messages = Vec::with_capacity(rows.len());
    for (offset, row) in rows.into_iter().enumerate() {
        let ordinal: i64 = row.try_get("ordinal")?;
        if ordinal
            != first_expected
                + i64::try_from(offset).map_err(|_| AppError::new(AppErrorCode::DatabaseError))?
        {
            return Err(AppError::new(AppErrorCode::DatabaseError));
        }
        let expected_role = if ordinal % 2 == 0 {
            UnifiedRole::User
        } else {
            UnifiedRole::Assistant
        };
        let role = match row.try_get::<String, _>("role")?.as_str() {
            "user" => UnifiedRole::User,
            "assistant" => UnifiedRole::Assistant,
            _ => return Err(AppError::new(AppErrorCode::DatabaseError)),
        };
        if role != expected_role {
            return Err(AppError::new(AppErrorCode::DatabaseError));
        }
        let content: String = row.try_get("content")?;
        if content.trim().is_empty() || content.len() > crate::domain::MAX_LEARNING_OUTPUT_BYTES {
            return Err(AppError::new(AppErrorCode::DatabaseError));
        }
        messages.push(UnifiedMessage {
            role,
            content: strip_historical_citation_ids(&content),
        });
    }
    Ok((expected_next_ordinal, messages))
}

fn retain_recent_history_pairs(
    messages: Vec<UnifiedMessage>,
    budget_tokens: u64,
) -> Vec<UnifiedMessage> {
    let pairs = messages.chunks_exact(2).collect::<Vec<_>>();
    let mut retained_reversed = Vec::new();
    let mut used = 0u64;
    for pair in pairs.into_iter().rev() {
        let cost = pair
            .iter()
            .fold(2 * MESSAGE_FRAMING_TOKENS, |total, message| {
                total.saturating_add(conservative_token_count(&message.content))
            });
        if used.saturating_add(cost) > budget_tokens {
            break;
        }
        used = used.saturating_add(cost);
        retained_reversed.push((pair[0].clone(), pair[1].clone()));
    }
    retained_reversed.reverse();
    retained_reversed
        .into_iter()
        .flat_map(|(user, assistant)| [user, assistant])
        .collect()
}

pub(crate) fn strip_historical_citation_ids(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut result = String::with_capacity(value.len());
    let mut cursor = 0usize;
    while cursor < bytes.len() {
        if cursor + 4 <= bytes.len() && &bytes[cursor..cursor + 4] == b"TL-C" {
            let start = cursor;
            cursor += 4;
            let digits = cursor;
            while cursor < bytes.len() && bytes[cursor].is_ascii_digit() {
                cursor += 1;
            }
            if cursor > digits {
                result.push_str("[historical citation omitted]");
                continue;
            }
            cursor = start;
        }
        let character = value[cursor..]
            .chars()
            .next()
            .expect("cursor is on a UTF-8 boundary");
        result.push(character);
        cursor += character.len_utf8();
    }
    result
}

fn parse_uuid(value: String) -> AppResult<Uuid> {
    Uuid::parse_str(&value).map_err(|_| AppError::new(AppErrorCode::DatabaseError))
}

#[cfg(test)]
#[path = "history_test.rs"]
mod tests;
