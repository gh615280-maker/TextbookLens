use std::{collections::BTreeMap, fmt};

use sqlx::{Row, Sqlite, Transaction};
use uuid::Uuid;

use crate::{
    domain::{
        Citation, CitationReviewStatus, ContentSource, DocumentLocator, LearningAction,
        NormalizedRect,
    },
    errors::{AppError, AppErrorCode, AppResult},
};

pub const MAX_LEARNING_QUESTION_CODE_POINTS: usize = 16_384;
pub const MAX_LEARNING_QUESTION_BYTES: usize = 64 * 1024;
pub const MAX_LEARNING_ANSWER_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_LEARNING_ANSWER_CODE_POINTS: usize = 1_048_576;
pub const MAX_LEARNING_CITATIONS: usize = 256;
pub const MAX_LEARNING_CITATIONS_JSON_BYTES: usize = 1024 * 1024;
pub const MAX_LEARNING_MODEL_ID_CODE_POINTS: usize = 256;

#[derive(Clone)]
pub struct CompletedAssistantMessage {
    pub provider_profile_id: Uuid,
    pub model_id: String,
    pub answer: String,
    pub available_citations: Vec<Citation>,
}

impl fmt::Debug for CompletedAssistantMessage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CompletedAssistantMessage")
            .field("provider_profile_id", &"<redacted>")
            .field("model_id", &"<redacted>")
            .field("answer_bytes", &self.answer.len())
            .field("available_citation_count", &self.available_citations.len())
            .finish()
    }
}

pub(crate) fn validate_question(value: &str) -> AppResult<()> {
    if value.trim().is_empty()
        || value.len() > MAX_LEARNING_QUESTION_BYTES
        || value.chars().count() > MAX_LEARNING_QUESTION_CODE_POINTS
        || contains_disallowed_control(value)
    {
        return Err(invalid_input());
    }
    Ok(())
}

pub(crate) fn validate_assistant(input: &CompletedAssistantMessage) -> AppResult<()> {
    if input.model_id.trim() != input.model_id
        || input.model_id.is_empty()
        || input.model_id.chars().count() > MAX_LEARNING_MODEL_ID_CODE_POINTS
        || input.model_id.chars().any(char::is_control)
        || input.answer.trim().is_empty()
        || input.answer.len() > MAX_LEARNING_ANSWER_BYTES
        || input.answer.chars().count() > MAX_LEARNING_ANSWER_CODE_POINTS
        || contains_disallowed_control(&input.answer)
        || input.available_citations.len() > MAX_LEARNING_CITATIONS
    {
        return Err(invalid_input());
    }
    Ok(())
}

pub(crate) async fn validated_used_citations_json(
    transaction: &mut Transaction<'_, Sqlite>,
    book_id: Uuid,
    answer: &str,
    available: &[Citation],
) -> AppResult<String> {
    let mut by_id = BTreeMap::new();
    for (index, citation) in available.iter().enumerate() {
        let expected_id = format!("TL-C{}", index + 1);
        if citation.id != expected_id
            || by_id.insert(citation.id.clone(), citation).is_some()
            || citation.book_id != book_id
            || !citation.quoteable
        {
            return Err(invalid_input());
        }
        revalidate_citation(transaction, book_id, citation).await?;
    }

    let cited_ids = extract_model_citation_ids(answer)?;
    let mut used = Vec::with_capacity(cited_ids.len());
    for id in cited_ids {
        let citation = by_id.get(&id).ok_or_else(invalid_input)?;
        used.push((*citation).clone());
    }
    let json = serde_json::to_string(&used).map_err(|_| database_error())?;
    if json.len() > MAX_LEARNING_CITATIONS_JSON_BYTES {
        return Err(invalid_input());
    }
    Ok(json)
}

pub(crate) async fn insert_user_message(
    transaction: &mut Transaction<'_, Sqlite>,
    message_id: Uuid,
    conversation_id: Uuid,
    ordinal: u32,
    action: LearningAction,
    content: &str,
    timestamp: &str,
) -> AppResult<()> {
    let result = sqlx::query(
        "INSERT INTO messages (id, conversation_id, ordinal, role, action, content, created_at) VALUES (?, ?, ?, 'user', ?, ?, ?)",
    )
    .bind(message_id.to_string())
    .bind(conversation_id.to_string())
    .bind(i64::from(ordinal))
    .bind(action_name(action))
    .bind(content)
    .bind(timestamp)
    .execute(&mut **transaction)
    .await?;
    if result.rows_affected() != 1 {
        return Err(database_error());
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn insert_assistant_message(
    transaction: &mut Transaction<'_, Sqlite>,
    message_id: Uuid,
    conversation_id: Uuid,
    ordinal: u32,
    action: LearningAction,
    input: &CompletedAssistantMessage,
    citations_json: &str,
    timestamp: &str,
) -> AppResult<()> {
    let result = sqlx::query(
        "INSERT INTO messages (id, conversation_id, ordinal, role, action, content, provider_id, model_id, citations_json, created_at) VALUES (?, ?, ?, 'assistant', ?, ?, ?, ?, ?, ?)",
    )
    .bind(message_id.to_string())
    .bind(conversation_id.to_string())
    .bind(i64::from(ordinal))
    .bind(action_name(action))
    .bind(&input.answer)
    .bind(input.provider_profile_id.to_string())
    .bind(&input.model_id)
    .bind(citations_json)
    .bind(timestamp)
    .execute(&mut **transaction)
    .await?;
    if result.rows_affected() != 1 {
        return Err(database_error());
    }
    Ok(())
}

pub(crate) fn extract_model_citation_ids(answer: &str) -> AppResult<Vec<String>> {
    let bytes = answer.as_bytes();
    let mut result = Vec::new();
    let mut index = 0usize;
    while index + 4 <= bytes.len() {
        if &bytes[index..index + 4] != b"TL-C" {
            index += 1;
            continue;
        }
        let token_start = index;
        index += 4;
        let digit_start = index;
        while index < bytes.len() && bytes[index].is_ascii_digit() {
            index += 1;
        }
        if digit_start == index {
            continue;
        }
        let left_is_identifier = token_start > 0
            && (bytes[token_start - 1].is_ascii_alphanumeric() || bytes[token_start - 1] == b'_');
        let right_is_identifier =
            index < bytes.len() && (bytes[index].is_ascii_alphanumeric() || bytes[index] == b'_');
        let digits = &answer[digit_start..index];
        if left_is_identifier
            || right_is_identifier
            || digits.starts_with('0')
            || digits.parse::<u32>().is_err()
        {
            return Err(invalid_input());
        }
        let id = answer[token_start..index].to_owned();
        if !result.iter().any(|existing| existing == &id) {
            if result.len() == MAX_LEARNING_CITATIONS {
                return Err(invalid_input());
            }
            result.push(id);
        }
    }
    Ok(result)
}

async fn revalidate_citation(
    transaction: &mut Transaction<'_, Sqlite>,
    book_id: Uuid,
    citation: &Citation,
) -> AppResult<()> {
    if citation.label.trim().is_empty()
        || citation.label.chars().count() > 256
        || citation
            .label
            .chars()
            .any(|value| value.is_control() && value != '\t')
    {
        return Err(invalid_input());
    }
    match citation.source {
        ContentSource::LocalText => {
            if citation.review_status != CitationReviewStatus::NotRequired {
                return Err(invalid_input());
            }
            let section_id = citation.section_id.ok_or_else(invalid_input)?;
            validate_local_locator(transaction, book_id, section_id, &citation.locator).await
        }
        ContentSource::AiTranscribed | ContentSource::UserCorrected => {
            if citation.section_id.is_some() {
                let section_exists: bool = sqlx::query_scalar(
                    "SELECT EXISTS(SELECT 1 FROM sections WHERE id = ? AND book_id = ?)",
                )
                .bind(citation.section_id.map(|id| id.to_string()))
                .bind(book_id.to_string())
                .fetch_one(&mut **transaction)
                .await?;
                if !section_exists {
                    return Err(invalid_input());
                }
            }
            validate_indexed_locator(transaction, book_id, citation).await
        }
        ContentSource::AiDescription => Err(invalid_input()),
    }
}

async fn validate_local_locator(
    transaction: &mut Transaction<'_, Sqlite>,
    book_id: Uuid,
    section_id: Uuid,
    locator: &DocumentLocator,
) -> AppResult<()> {
    let row = sqlx::query(
        "SELECT b.format, s.locator_json FROM sections s JOIN books b ON b.id = s.book_id WHERE s.id = ? AND s.book_id = ? AND b.import_status = 'ready'",
    )
    .bind(section_id.to_string())
    .bind(book_id.to_string())
    .fetch_optional(&mut **transaction)
    .await?
    .ok_or_else(invalid_input)?;
    let format: String = row.try_get("format")?;
    let section_locator: DocumentLocator =
        serde_json::from_str(&row.try_get::<String, _>("locator_json")?)
            .map_err(|_| database_error())?;
    let valid = match (locator, section_locator, format.as_str()) {
        (
            DocumentLocator::Pdf {
                start_page,
                end_page,
                rects_by_page,
            },
            DocumentLocator::Pdf {
                start_page: section_start,
                end_page: section_end,
                ..
            },
            "pdf",
        ) => {
            *start_page > 0
                && start_page <= end_page
                && *start_page >= section_start
                && *end_page <= section_end
                && rects_by_page.as_ref().is_none_or(|pages| {
                    pages.iter().all(|(page, rects)| {
                        *page >= *start_page
                            && *page <= *end_page
                            && !rects.is_empty()
                            && rects.iter().all(valid_rect)
                    })
                })
        }
        (
            DocumentLocator::Epub {
                cfi,
                section_id: locator_section,
            },
            DocumentLocator::Epub {
                section_id: stored_section,
                ..
            },
            "epub",
        ) => {
            *locator_section == section_id
                && stored_section == section_id
                && !cfi.trim().is_empty()
                && cfi.chars().count() <= 4_096
                && !contains_disallowed_control(cfi)
        }
        (
            DocumentLocator::Docx {
                start_block_id,
                start_offset,
                end_block_id,
                end_offset,
            },
            DocumentLocator::Docx { .. },
            "docx",
        ) => {
            let start =
                load_block_bounds(transaction, book_id, section_id, *start_block_id).await?;
            let end = load_block_bounds(transaction, book_id, section_id, *end_block_id).await?;
            matches!(
                (start, end),
                (Some((start_ordinal, start_length)), Some((end_ordinal, end_length)))
                    if start_ordinal <= end_ordinal
                        && *start_offset <= start_length
                        && *end_offset <= end_length
                        && (start_ordinal != end_ordinal || start_offset <= end_offset)
            )
        }
        _ => false,
    };
    if valid { Ok(()) } else { Err(invalid_input()) }
}

async fn load_block_bounds(
    transaction: &mut Transaction<'_, Sqlite>,
    book_id: Uuid,
    section_id: Uuid,
    block_id: Uuid,
) -> AppResult<Option<(i64, u32)>> {
    let row = sqlx::query(
        "SELECT ordinal, plain_text FROM blocks WHERE id = ? AND book_id = ? AND section_id = ?",
    )
    .bind(block_id.to_string())
    .bind(book_id.to_string())
    .bind(section_id.to_string())
    .fetch_optional(&mut **transaction)
    .await?;
    row.map(|row| {
        let length = u32::try_from(row.try_get::<String, _>("plain_text")?.chars().count())
            .map_err(|_| database_error())?;
        Ok((row.try_get("ordinal")?, length))
    })
    .transpose()
}

async fn validate_indexed_locator(
    transaction: &mut Transaction<'_, Sqlite>,
    book_id: Uuid,
    citation: &Citation,
) -> AppResult<()> {
    let locator_json = serde_json::to_string(&citation.locator).map_err(|_| database_error())?;
    if locator_json.len() > 16_384 {
        return Err(invalid_input());
    }
    let source = match citation.source {
        ContentSource::AiTranscribed => "ai_transcribed",
        ContentSource::UserCorrected => "user_corrected",
        _ => return Err(invalid_input()),
    };
    let rows = sqlx::query(
        "SELECT p.status AS page_status, p.review_reason_code, c.correction_id FROM index_search_chunks c JOIN index_pages p ON p.id = c.page_id AND p.book_id = c.book_id AND p.content_version = c.content_version AND p.status IN ('indexed', 'needs_review') LEFT JOIN index_corrections correction ON correction.id = c.correction_id AND correction.page_id = c.page_id AND correction.book_id = c.book_id WHERE c.book_id = ? AND c.source = ? AND c.locator_json = ? AND (c.source != 'user_corrected' OR (correction.id IS NOT NULL AND correction.conflict_state = 'active' AND correction.target_content_version = c.content_version)) AND NOT (c.source = 'ai_transcribed' AND EXISTS (SELECT 1 FROM index_corrections active_correction WHERE active_correction.book_id = c.book_id AND active_correction.page_id = c.page_id AND active_correction.target_block_id = c.block_id AND active_correction.target_content_version = c.content_version AND active_correction.value_kind = 'text' AND active_correction.conflict_state = 'active')) AND NOT (c.source IN ('ai_transcribed', 'user_corrected') AND EXISTS (SELECT 1 FROM index_corrections conflicted_correction WHERE conflicted_correction.book_id = c.book_id AND conflicted_correction.page_id = c.page_id AND conflicted_correction.target_block_id = c.block_id AND conflicted_correction.conflict_state = 'conflict'))",
    )
    .bind(book_id.to_string())
    .bind(source)
    .bind(locator_json)
    .fetch_all(&mut **transaction)
    .await?;
    let valid = rows.iter().any(|row| {
        if citation.source == ContentSource::UserCorrected {
            return citation.review_status == CitationReviewStatus::UserCorrected
                && row
                    .try_get::<Option<String>, _>("correction_id")
                    .ok()
                    .flatten()
                    .is_some();
        }
        let needs_review = row
            .try_get::<String, _>("page_status")
            .is_ok_and(|status| status == "needs_review")
            || row
                .try_get::<Option<String>, _>("review_reason_code")
                .is_ok_and(|value| value.is_some());
        citation.review_status
            == if needs_review {
                CitationReviewStatus::NeedsReview
            } else {
                CitationReviewStatus::Indexed
            }
    });
    if valid { Ok(()) } else { Err(invalid_input()) }
}

fn valid_rect(rect: &NormalizedRect) -> bool {
    [rect.x, rect.y, rect.width, rect.height]
        .iter()
        .all(|value| value.is_finite())
        && rect.x >= 0.0
        && rect.y >= 0.0
        && rect.width > 0.0
        && rect.height > 0.0
        && rect.x + rect.width <= 1.0
        && rect.y + rect.height <= 1.0
}

pub(crate) const fn action_name(action: LearningAction) -> &'static str {
    match action {
        LearningAction::Explain => "explain",
        LearningAction::Example => "example",
        LearningAction::Derive => "derive",
        LearningAction::Translate => "translate",
        LearningAction::Ask => "ask",
        LearningAction::Continue => "continue",
        LearningAction::Overview => "overview",
    }
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
#[path = "messages_test.rs"]
mod tests;
