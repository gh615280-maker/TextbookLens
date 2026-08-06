use std::collections::{BTreeMap, BTreeSet};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};
use sqlx::{Row, SqlitePool, sqlite::SqliteRow};
use uuid::Uuid;

use crate::{
    domain::{
        BookFormat, Citation, ContentSource, DocumentLocator, LearningOverview,
        LearningOverviewActivity, LearningOverviewSection, LearningOverviewSource,
        LearningOverviewSourceSummary, stable_index_search_chunk_id, stable_section_id,
        validate_teaching_instruction,
    },
    errors::{AppError, AppErrorCode, AppResult},
};

use super::messages::{
    MAX_LEARNING_ANSWER_BYTES, MAX_LEARNING_ANSWER_CODE_POINTS, MAX_LEARNING_CITATIONS,
    MAX_LEARNING_CITATIONS_JSON_BYTES, MAX_LEARNING_MODEL_ID_CODE_POINTS,
    MAX_LEARNING_QUESTION_BYTES, MAX_LEARNING_QUESTION_CODE_POINTS, parse_database_timestamp,
};

pub const OVERVIEW_READ_QUERY_COUNT: u8 = 11;
const MAX_OVERVIEW_SECTIONS: usize = 100_000;
const MAX_OVERVIEW_CONVERSATIONS: usize = 100_000;
const MAX_OVERVIEW_MESSAGES: usize = 1_000_000;
const MAX_OVERVIEW_CORRECTIONS: usize = 100_000;
const MAX_SECTION_TITLE_CODE_POINTS: usize = 4_096;
const MAX_SECTION_TITLE_BYTES: usize = 16 * 1024;
const MAX_SECTION_LOCATOR_JSON_BYTES: usize = 16 * 1024;
const MAX_STORED_PATH_BYTES: i64 = 16 * 1024;

#[async_trait]
pub(crate) trait OverviewReadObserver: Send + Sync {
    async fn after_query(&self, query_count: u8);
}

struct NoopOverviewReadObserver;

#[async_trait]
impl OverviewReadObserver for NoopOverviewReadObserver {
    async fn after_query(&self, _query_count: u8) {}
}

pub async fn get_learning_overview(
    pool: &SqlitePool,
    book_id: Uuid,
) -> AppResult<LearningOverview> {
    get_learning_overview_observed(pool, book_id, &NoopOverviewReadObserver).await
}

pub(crate) async fn get_learning_overview_observed<O>(
    pool: &SqlitePool,
    book_id: Uuid,
    observer: &O,
) -> AppResult<LearningOverview>
where
    O: OverviewReadObserver + ?Sized,
{
    let mut transaction = pool.begin().await?;
    let mut query_count = 0_u8;

    let book = load_book_and_teaching(&mut transaction, book_id).await?;
    record_query(&mut query_count, observer).await?;

    verify_quick_check(&mut transaction).await?;
    record_query(&mut query_count, observer).await?;

    verify_foreign_keys(&mut transaction).await?;
    record_query(&mut query_count, observer).await?;

    verify_target_ownership(&mut transaction, book_id).await?;
    record_query(&mut query_count, observer).await?;

    let (mut sections, section_indexes) =
        load_sections(&mut transaction, book_id, &book.format).await?;
    record_query(&mut query_count, observer).await?;

    let mut sources = LearningOverviewSource::ALL.map(LearningOverviewSourceSummary::empty);
    load_local_content(
        &mut transaction,
        book_id,
        &section_indexes,
        &mut sections,
        &mut sources,
    )
    .await?;
    record_query(&mut query_count, observer).await?;

    load_indexed_content(&mut transaction, book_id, &mut sources).await?;
    record_query(&mut query_count, observer).await?;

    verify_active_correction_provenance(
        &mut transaction,
        book_id,
        sources[LearningOverviewSource::UserCorrected.index()].item_count,
    )
    .await?;
    record_query(&mut query_count, observer).await?;

    let user_note_count = load_notes(
        &mut transaction,
        book_id,
        &section_indexes,
        &mut sections,
        &mut sources,
    )
    .await?;
    record_query(&mut query_count, observer).await?;

    let conversations = load_conversations(&mut transaction, book_id, &section_indexes).await?;
    record_query(&mut query_count, observer).await?;

    let activity = load_messages_and_finish_history(
        &mut transaction,
        book_id,
        &book.format,
        conversations,
        &section_indexes,
        &mut sections,
        &mut sources,
        user_note_count,
    )
    .await?;
    record_query(&mut query_count, observer).await?;

    if query_count != OVERVIEW_READ_QUERY_COUNT {
        return Err(database_error());
    }
    let section_count = to_u32(sections.len())?;
    transaction.commit().await?;
    Ok(LearningOverview {
        book_id,
        format: book.format,
        teaching_instruction_configured: book.teaching_instruction_configured,
        section_count,
        sections,
        sources: sources.into_iter().collect(),
        activity,
    })
}

struct BookSnapshot {
    format: BookFormat,
    teaching_instruction_configured: bool,
}

async fn load_book_and_teaching(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    book_id: Uuid,
) -> AppResult<BookSnapshot> {
    let row = sqlx::query(
        "SELECT id, format, import_status, sha256, length(CAST(stored_path AS BLOB)) AS stored_path_bytes, (SELECT COUNT(*) FROM teaching_preferences) AS teaching_count, (SELECT instruction FROM teaching_preferences WHERE id = 1) AS teaching_instruction, (SELECT revision FROM teaching_preferences WHERE id = 1) AS teaching_revision FROM books WHERE id = ?",
    )
    .bind(book_id.to_string())
    .fetch_optional(&mut **transaction)
    .await?
    .ok_or_else(|| AppError::new(AppErrorCode::NotFound))?;
    if parse_uuid(row.try_get::<String, _>("id")?)? != book_id {
        return Err(database_error());
    }
    let status: String = row.try_get("import_status")?;
    if status != "ready" {
        return Err(AppError::new(AppErrorCode::BookNotReady));
    }
    let sha256 = row
        .try_get::<Option<String>, _>("sha256")?
        .ok_or_else(database_error)?;
    let stored_path_bytes = row
        .try_get::<Option<i64>, _>("stored_path_bytes")?
        .ok_or_else(database_error)?;
    if !valid_sha256(&sha256) || !(1..=MAX_STORED_PATH_BYTES).contains(&stored_path_bytes) {
        return Err(database_error());
    }
    if row.try_get::<i64, _>("teaching_count")? != 1 {
        return Err(database_error());
    }
    let teaching_instruction = row
        .try_get::<Option<String>, _>("teaching_instruction")?
        .ok_or_else(database_error)?;
    validate_teaching_instruction(&teaching_instruction).map_err(|_| database_error())?;
    if !teaching_instruction.is_empty() && teaching_instruction.chars().all(char::is_whitespace) {
        return Err(database_error());
    }
    u64::try_from(
        row.try_get::<Option<i64>, _>("teaching_revision")?
            .ok_or_else(database_error)?,
    )
    .map_err(|_| database_error())?;

    Ok(BookSnapshot {
        format: parse_book_format(&row.try_get::<String, _>("format")?)?,
        teaching_instruction_configured: !teaching_instruction.is_empty(),
    })
}

async fn verify_quick_check(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
) -> AppResult<()> {
    let rows = sqlx::query_scalar::<_, String>("PRAGMA quick_check(1)")
        .fetch_all(&mut **transaction)
        .await?;
    if rows.as_slice() == ["ok"] {
        Ok(())
    } else {
        Err(database_error())
    }
}

async fn verify_foreign_keys(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
) -> AppResult<()> {
    let rows = sqlx::query("PRAGMA foreign_key_check")
        .fetch_all(&mut **transaction)
        .await?;
    if rows.is_empty() {
        Ok(())
    } else {
        Err(database_error())
    }
}

async fn verify_target_ownership(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    book_id: Uuid,
) -> AppResult<()> {
    let row = sqlx::query(
        r#"
WITH target(book_id) AS (VALUES (?))
SELECT
  (SELECT COUNT(*) FROM sections child
     LEFT JOIN sections parent ON parent.id = child.parent_id
   WHERE child.book_id = (SELECT book_id FROM target)
     AND child.parent_id IS NOT NULL
     AND (parent.id IS NULL OR parent.book_id != child.book_id OR parent.id = child.id)) AS invalid_sections,
  (SELECT COUNT(*) FROM blocks block
     LEFT JOIN sections section ON section.id = block.section_id
   WHERE block.book_id = (SELECT book_id FROM target)
     AND (section.id IS NULL OR section.book_id != block.book_id)) AS invalid_blocks,
  (SELECT COUNT(*) FROM search_chunks chunk
     LEFT JOIN sections section ON section.id = chunk.section_id
   WHERE chunk.book_id = (SELECT book_id FROM target)
     AND (section.id IS NULL OR section.book_id != chunk.book_id)) AS invalid_local_chunks,
  (SELECT COUNT(*) FROM index_runs run
     LEFT JOIN books book ON book.id = run.book_id
   WHERE run.book_id = (SELECT book_id FROM target)
     AND (book.id IS NULL OR book.sha256 IS NULL OR book.sha256 != run.source_sha256)) AS invalid_runs,
  (SELECT COUNT(*) FROM index_pages page
     LEFT JOIN index_runs run ON run.id = page.run_id
   WHERE page.book_id = (SELECT book_id FROM target)
     AND (run.id IS NULL OR run.book_id != page.book_id)) AS invalid_pages,
  (SELECT COUNT(*) FROM index_page_blocks block
     LEFT JOIN index_pages page ON page.id = block.page_id
   WHERE block.book_id = (SELECT book_id FROM target)
     AND (page.id IS NULL OR page.book_id != block.book_id OR page.run_id != block.run_id)) AS invalid_page_blocks,
  (SELECT COUNT(*) FROM index_corrections correction
     LEFT JOIN index_pages page ON page.id = correction.page_id
     LEFT JOIN index_page_blocks block ON block.id = correction.target_block_id
   WHERE correction.book_id = (SELECT book_id FROM target)
     AND (page.id IS NULL OR page.book_id != correction.book_id
       OR block.id IS NULL OR block.book_id != correction.book_id
       OR block.page_id != correction.page_id)) AS invalid_corrections,
  (SELECT COUNT(*) FROM index_search_chunks chunk
     LEFT JOIN index_pages page ON page.id = chunk.page_id
     LEFT JOIN index_page_blocks block ON block.id = chunk.block_id
     LEFT JOIN index_corrections correction ON correction.id = chunk.correction_id
   WHERE chunk.book_id = (SELECT book_id FROM target)
     AND (page.id IS NULL OR page.book_id != chunk.book_id
       OR block.id IS NULL OR block.book_id != chunk.book_id OR block.page_id != chunk.page_id
       OR (chunk.correction_id IS NOT NULL AND (
         correction.id IS NULL OR correction.book_id != chunk.book_id
         OR correction.page_id != chunk.page_id OR correction.target_block_id != chunk.block_id)))) AS invalid_index_chunks,
  (SELECT COUNT(*) FROM conversations conversation
     LEFT JOIN sections section ON section.id = conversation.section_id
   WHERE conversation.book_id = (SELECT book_id FROM target)
     AND ((conversation.scope = 'selection' AND (section.id IS NULL OR section.book_id != conversation.book_id))
       OR (conversation.scope = 'book' AND conversation.section_id IS NOT NULL))) AS invalid_conversations,
  (SELECT COUNT(*) FROM annotations annotation
     LEFT JOIN sections section ON section.id = annotation.section_id
     LEFT JOIN conversations conversation ON conversation.id = annotation.conversation_id
   WHERE (annotation.book_id = (SELECT book_id FROM target)
       OR conversation.book_id = (SELECT book_id FROM target))
     AND ((annotation.section_id IS NOT NULL AND (section.id IS NULL OR section.book_id != annotation.book_id))
       OR (annotation.kind = 'note' AND annotation.section_id IS NULL)
       OR (annotation.kind = 'ai_conversation' AND (
         conversation.id IS NULL OR conversation.book_id != annotation.book_id
         OR conversation.section_id IS NOT annotation.section_id)))) AS invalid_annotations
"#,
    )
    .bind(book_id.to_string())
    .fetch_one(&mut **transaction)
    .await?;
    for column in [
        "invalid_sections",
        "invalid_blocks",
        "invalid_local_chunks",
        "invalid_runs",
        "invalid_pages",
        "invalid_page_blocks",
        "invalid_corrections",
        "invalid_index_chunks",
        "invalid_conversations",
        "invalid_annotations",
    ] {
        if row.try_get::<i64, _>(column)? != 0 {
            return Err(database_error());
        }
    }
    Ok(())
}

async fn load_sections(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    book_id: Uuid,
    format: &BookFormat,
) -> AppResult<(Vec<LearningOverviewSection>, BTreeMap<Uuid, usize>)> {
    let limit = i64::try_from(MAX_OVERVIEW_SECTIONS + 1).map_err(|_| database_error())?;
    let rows = sqlx::query(
        "SELECT id, book_id, parent_id, ordinal, title, locator_json FROM sections WHERE book_id = ? ORDER BY ordinal, id LIMIT ?",
    )
    .bind(book_id.to_string())
    .bind(limit)
    .fetch_all(&mut **transaction)
    .await?;
    if rows.len() > MAX_OVERVIEW_SECTIONS {
        return Err(database_error());
    }

    let mut sections = Vec::with_capacity(rows.len());
    let mut indexes = BTreeMap::new();
    for row in rows {
        let id = parse_uuid(row.try_get::<String, _>("id")?)?;
        if parse_uuid(row.try_get::<String, _>("book_id")?)? != book_id {
            return Err(database_error());
        }
        let ordinal = to_u32_i64(row.try_get::<i64, _>("ordinal")?)?;
        if id != stable_section_id(book_id, ordinal) {
            return Err(database_error());
        }
        let parent_id = row
            .try_get::<Option<String>, _>("parent_id")?
            .map(parse_uuid)
            .transpose()?;
        let title: String = row.try_get("title")?;
        if !valid_section_title(&title) {
            return Err(database_error());
        }
        let locator_json: String = row.try_get("locator_json")?;
        if locator_json.len() > MAX_SECTION_LOCATOR_JSON_BYTES {
            return Err(database_error());
        }
        let locator: DocumentLocator =
            serde_json::from_str(&locator_json).map_err(|_| database_error())?;
        if !locator_matches_format_and_section(&locator, format, id) {
            return Err(database_error());
        }
        let index = sections.len();
        if indexes.insert(id, index).is_some() {
            return Err(database_error());
        }
        sections.push(LearningOverviewSection {
            id,
            parent_id,
            ordinal,
            title,
            local_text_item_count: 0,
            user_note_count: 0,
            completed_conversation_count: 0,
            completed_exchange_count: 0,
        });
    }
    if sections
        .windows(2)
        .any(|pair| pair[0].ordinal >= pair[1].ordinal)
        || sections.iter().any(|section| {
            section.parent_id.is_some_and(|parent_id| {
                parent_id == section.id || !indexes.contains_key(&parent_id)
            })
        })
    {
        return Err(database_error());
    }
    Ok((sections, indexes))
}

async fn load_local_content(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    book_id: Uuid,
    section_indexes: &BTreeMap<Uuid, usize>,
    sections: &mut [LearningOverviewSection],
    sources: &mut [LearningOverviewSourceSummary; 6],
) -> AppResult<()> {
    let rows = sqlx::query(
        "SELECT section_id, COUNT(*) AS item_count, SUM(CASE WHEN kind NOT IN ('heading', 'paragraph', 'list', 'table', 'caption', 'equation') OR length(trim(plain_text, char(9) || char(10) || char(11) || char(12) || char(13) || char(32))) = 0 OR instr(plain_text, char(0)) != 0 OR instr(plain_text, char(13)) != 0 THEN 1 ELSE 0 END) AS invalid_count FROM blocks WHERE book_id = ? GROUP BY section_id ORDER BY section_id",
    )
    .bind(book_id.to_string())
    .fetch_all(&mut **transaction)
    .await?;
    let summary = &mut sources[LearningOverviewSource::LocalText.index()];
    for row in rows {
        if row.try_get::<i64, _>("invalid_count")? != 0 {
            return Err(database_error());
        }
        let section_id = parse_uuid(row.try_get::<String, _>("section_id")?)?;
        let index = *section_indexes
            .get(&section_id)
            .ok_or_else(database_error)?;
        let count = to_u32_i64(row.try_get::<i64, _>("item_count")?)?;
        if count == 0 {
            return Err(database_error());
        }
        sections[index].local_text_item_count = count;
        checked_add_assign(&mut summary.item_count, count)?;
        checked_add_assign(&mut summary.covered_section_count, 1)?;
    }
    Ok(())
}

async fn load_indexed_content(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    book_id: Uuid,
    sources: &mut [LearningOverviewSourceSummary; 6],
) -> AppResult<()> {
    let rows = sqlx::query(
        r#"
SELECT chunk.source, COUNT(*) AS item_count, COUNT(DISTINCT chunk.page_id) AS page_count
FROM index_search_chunks chunk
JOIN index_pages page ON page.id = chunk.page_id
  AND page.book_id = chunk.book_id
  AND page.content_version = chunk.content_version
  AND page.status IN ('indexed', 'needs_review')
LEFT JOIN index_corrections correction ON correction.id = chunk.correction_id
  AND correction.page_id = chunk.page_id
  AND correction.book_id = chunk.book_id
WHERE chunk.book_id = ?
  AND (chunk.source != 'user_corrected' OR (
    correction.id IS NOT NULL
    AND correction.conflict_state = 'active'
    AND correction.target_content_version = chunk.content_version))
  AND NOT (chunk.source = 'ai_transcribed' AND EXISTS (
    SELECT 1 FROM index_corrections active_correction
    WHERE active_correction.book_id = chunk.book_id
      AND active_correction.page_id = chunk.page_id
      AND active_correction.target_block_id = chunk.block_id
      AND active_correction.target_content_version = chunk.content_version
      AND active_correction.value_kind = 'text'
      AND active_correction.conflict_state = 'active'))
  AND NOT (chunk.source IN ('ai_transcribed', 'user_corrected') AND EXISTS (
    SELECT 1 FROM index_corrections conflicted_correction
    WHERE conflicted_correction.book_id = chunk.book_id
      AND conflicted_correction.page_id = chunk.page_id
      AND conflicted_correction.target_block_id = chunk.block_id
      AND conflicted_correction.conflict_state = 'conflict'))
GROUP BY chunk.source
ORDER BY CASE chunk.source
  WHEN 'ai_transcribed' THEN 0
  WHEN 'ai_description' THEN 1
  WHEN 'user_corrected' THEN 2
  ELSE 3 END
"#,
    )
    .bind(book_id.to_string())
    .fetch_all(&mut **transaction)
    .await?;
    let mut seen = BTreeSet::new();
    for row in rows {
        let source = ContentSource::from_database(&row.try_get::<String, _>("source")?)
            .ok_or_else(database_error)?;
        let overview_source = match source {
            ContentSource::AiTranscribed => LearningOverviewSource::AiTranscribed,
            ContentSource::AiDescription => LearningOverviewSource::AiDescription,
            ContentSource::UserCorrected => LearningOverviewSource::UserCorrected,
            ContentSource::LocalText => return Err(database_error()),
        };
        if !seen.insert(overview_source.index()) {
            return Err(database_error());
        }
        let summary = &mut sources[overview_source.index()];
        summary.item_count = to_u32_i64(row.try_get::<i64, _>("item_count")?)?;
        summary.covered_page_count = to_u32_i64(row.try_get::<i64, _>("page_count")?)?;
        if summary.item_count == 0
            || summary.covered_page_count == 0
            || summary.covered_page_count > summary.item_count
        {
            return Err(database_error());
        }
    }
    Ok(())
}

async fn verify_active_correction_provenance(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    book_id: Uuid,
    expected_count: u32,
) -> AppResult<()> {
    let limit = i64::try_from(MAX_OVERVIEW_CORRECTIONS + 1).map_err(|_| database_error())?;
    let rows = sqlx::query(
        r#"
SELECT chunk.id AS chunk_id, chunk.page_id, chunk.block_id, chunk.correction_id,
  chunk.ordinal AS chunk_ordinal, chunk.text AS chunk_text,
  block.ordinal AS block_ordinal, block.plain_text, block.latex,
  correction.id AS owned_correction_id, correction.value_kind,
  correction.original_value_sha256, correction.original_value, correction.corrected_value
FROM index_search_chunks chunk
JOIN index_pages page ON page.id = chunk.page_id
  AND page.book_id = chunk.book_id
  AND page.content_version = chunk.content_version
  AND page.status IN ('indexed', 'needs_review')
JOIN index_page_blocks block ON block.id = chunk.block_id
  AND block.page_id = chunk.page_id
  AND block.book_id = chunk.book_id
  AND block.content_version = chunk.content_version
JOIN index_corrections correction ON correction.id = chunk.correction_id
  AND correction.page_id = chunk.page_id
  AND correction.book_id = chunk.book_id
  AND correction.target_block_id = chunk.block_id
  AND correction.target_content_version = chunk.content_version
  AND correction.conflict_state = 'active'
WHERE chunk.book_id = ? AND chunk.source = 'user_corrected'
  AND NOT EXISTS (
    SELECT 1 FROM index_corrections conflicted_correction
    WHERE conflicted_correction.book_id = chunk.book_id
      AND conflicted_correction.page_id = chunk.page_id
      AND conflicted_correction.target_block_id = chunk.block_id
      AND conflicted_correction.conflict_state = 'conflict')
ORDER BY page.page_number, chunk.ordinal, chunk.id
LIMIT ?
"#,
    )
    .bind(book_id.to_string())
    .bind(limit)
    .fetch_all(&mut **transaction)
    .await?;
    if rows.len() > MAX_OVERVIEW_CORRECTIONS || to_u32(rows.len())? != expected_count {
        return Err(database_error());
    }
    for row in rows {
        validate_correction_provenance(&row)?;
    }
    Ok(())
}

fn validate_correction_provenance(row: &SqliteRow) -> AppResult<()> {
    let page_id = parse_uuid(row.try_get::<String, _>("page_id")?)?;
    let block_id = parse_uuid(row.try_get::<String, _>("block_id")?)?;
    let correction_id = parse_uuid(
        row.try_get::<Option<String>, _>("correction_id")?
            .ok_or_else(database_error)?,
    )?;
    let owned_correction_id = parse_uuid(row.try_get::<String, _>("owned_correction_id")?)?;
    if correction_id != owned_correction_id {
        return Err(database_error());
    }
    let value_kind: String = row.try_get("value_kind")?;
    let offset = match value_kind.as_str() {
        "text" => 0_u32,
        "latex" => 1_u32,
        _ => return Err(database_error()),
    };
    let block_ordinal = to_u32_i64(row.try_get::<i64, _>("block_ordinal")?)?;
    let expected_ordinal = block_ordinal
        .checked_mul(4)
        .and_then(|value| value.checked_add(offset))
        .filter(|value| *value <= 100_000)
        .ok_or_else(database_error)?;
    let chunk_ordinal = to_u32_i64(row.try_get::<i64, _>("chunk_ordinal")?)?;
    let chunk_id = parse_uuid(row.try_get::<String, _>("chunk_id")?)?;
    if chunk_ordinal != expected_ordinal
        || chunk_id
            != stable_index_search_chunk_id(
                block_id,
                ContentSource::UserCorrected,
                expected_ordinal,
            )
        || correction_id != stable_correction_id(page_id, block_id, value_kind.as_str())?
    {
        return Err(database_error());
    }
    let original: String = row.try_get("original_value")?;
    let corrected: String = row.try_get("corrected_value")?;
    let chunk_text: String = row.try_get("chunk_text")?;
    let provider_value = match value_kind.as_str() {
        "text" => row.try_get::<Option<String>, _>("plain_text")?,
        "latex" => row.try_get::<Option<String>, _>("latex")?,
        _ => None,
    }
    .ok_or_else(database_error)?;
    let original_hash: String = row.try_get("original_value_sha256")?;
    if sha256_text(&original) != original_hash
        || provider_value != original
        || corrected != chunk_text
    {
        return Err(database_error());
    }
    Ok(())
}

async fn load_notes(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    book_id: Uuid,
    section_indexes: &BTreeMap<Uuid, usize>,
    sections: &mut [LearningOverviewSection],
    sources: &mut [LearningOverviewSourceSummary; 6],
) -> AppResult<u32> {
    let rows = sqlx::query(
        "SELECT section_id, COUNT(*) AS item_count, SUM(CASE WHEN section_id IS NULL OR revision < 1 OR revision > 1000000 OR length(trim(note_text, char(9) || char(10) || char(11) || char(12) || char(13) || char(32))) = 0 OR instr(note_text, char(0)) != 0 OR instr(note_text, char(13)) != 0 THEN 1 ELSE 0 END) AS invalid_count FROM annotations WHERE book_id = ? AND kind = 'note' GROUP BY section_id ORDER BY section_id",
    )
    .bind(book_id.to_string())
    .fetch_all(&mut **transaction)
    .await?;
    let summary = &mut sources[LearningOverviewSource::UserNote.index()];
    for row in rows {
        if row.try_get::<i64, _>("invalid_count")? != 0 {
            return Err(database_error());
        }
        let section_id = parse_uuid(
            row.try_get::<Option<String>, _>("section_id")?
                .ok_or_else(database_error)?,
        )?;
        let index = *section_indexes
            .get(&section_id)
            .ok_or_else(database_error)?;
        let count = to_u32_i64(row.try_get::<i64, _>("item_count")?)?;
        if count == 0 {
            return Err(database_error());
        }
        sections[index].user_note_count = count;
        checked_add_assign(&mut summary.item_count, count)?;
        checked_add_assign(&mut summary.covered_section_count, 1)?;
    }
    Ok(summary.item_count)
}

struct ConversationState {
    section_index: Option<usize>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    message_count: u32,
    exchange_count: u32,
    citation_count: u32,
    previous_action: Option<String>,
    previous_message_at: Option<DateTime<Utc>>,
}

async fn load_conversations(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    book_id: Uuid,
    section_indexes: &BTreeMap<Uuid, usize>,
) -> AppResult<BTreeMap<Uuid, ConversationState>> {
    let limit = i64::try_from(MAX_OVERVIEW_CONVERSATIONS + 1).map_err(|_| database_error())?;
    let rows = sqlx::query(
        r#"
SELECT conversation.id, conversation.book_id, conversation.section_id, conversation.scope,
  conversation.anchor_kind, conversation.anchor_json, conversation.selected_text,
  conversation.created_at, conversation.updated_at,
  COUNT(annotation.id) AS annotation_count,
  SUM(CASE WHEN annotation.id IS NOT NULL AND (
    annotation.book_id != conversation.book_id
    OR annotation.section_id IS NOT conversation.section_id
    OR annotation.kind != 'ai_conversation'
    OR annotation.anchor_json IS NOT conversation.anchor_json
    OR annotation.selected_text IS NOT conversation.selected_text) THEN 1 ELSE 0 END) AS annotation_mismatch_count
FROM conversations conversation
LEFT JOIN annotations annotation ON annotation.conversation_id = conversation.id
WHERE conversation.book_id = ?
GROUP BY conversation.id
ORDER BY conversation.id
LIMIT ?
"#,
    )
    .bind(book_id.to_string())
    .bind(limit)
    .fetch_all(&mut **transaction)
    .await?;
    if rows.len() > MAX_OVERVIEW_CONVERSATIONS {
        return Err(database_error());
    }
    let mut conversations = BTreeMap::new();
    for row in rows {
        let id = parse_uuid(row.try_get::<String, _>("id")?)?;
        if parse_uuid(row.try_get::<String, _>("book_id")?)? != book_id
            || row.try_get::<i64, _>("annotation_mismatch_count")? != 0
        {
            return Err(database_error());
        }
        let scope: String = row.try_get("scope")?;
        let section_id = row
            .try_get::<Option<String>, _>("section_id")?
            .map(parse_uuid)
            .transpose()?;
        let annotation_count = row.try_get::<i64, _>("annotation_count")?;
        let section_index = match scope.as_str() {
            "selection" => {
                if annotation_count != 1
                    || !matches!(
                        row.try_get::<Option<String>, _>("anchor_kind")?.as_deref(),
                        Some("text" | "region")
                    )
                    || row
                        .try_get::<Option<String>, _>("anchor_json")?
                        .as_deref()
                        .is_none_or(|json| {
                            json.len() > 128 * 1024
                                || serde_json::from_str::<serde_json::Value>(json).is_err()
                        })
                {
                    return Err(database_error());
                }
                Some(
                    *section_indexes
                        .get(&section_id.ok_or_else(database_error)?)
                        .ok_or_else(database_error)?,
                )
            }
            "book" => {
                if section_id.is_some()
                    || annotation_count != 0
                    || row.try_get::<Option<String>, _>("anchor_kind")?.is_some()
                    || row.try_get::<Option<String>, _>("anchor_json")?.is_some()
                    || row.try_get::<Option<String>, _>("selected_text")?.is_some()
                {
                    return Err(database_error());
                }
                None
            }
            _ => return Err(database_error()),
        };
        let created_at = parse_database_timestamp(&row.try_get::<String, _>("created_at")?)?;
        let updated_at = parse_database_timestamp(&row.try_get::<String, _>("updated_at")?)?;
        if updated_at < created_at
            || conversations
                .insert(
                    id,
                    ConversationState {
                        section_index,
                        created_at,
                        updated_at,
                        message_count: 0,
                        exchange_count: 0,
                        citation_count: 0,
                        previous_action: None,
                        previous_message_at: None,
                    },
                )
                .is_some()
        {
            return Err(database_error());
        }
    }
    Ok(conversations)
}

#[allow(clippy::too_many_arguments)]
async fn load_messages_and_finish_history(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    book_id: Uuid,
    format: &BookFormat,
    mut conversations: BTreeMap<Uuid, ConversationState>,
    section_indexes: &BTreeMap<Uuid, usize>,
    sections: &mut [LearningOverviewSection],
    sources: &mut [LearningOverviewSourceSummary; 6],
    user_note_count: u32,
) -> AppResult<LearningOverviewActivity> {
    let limit = i64::try_from(MAX_OVERVIEW_MESSAGES + 1).map_err(|_| database_error())?;
    let rows = sqlx::query(
        r#"
SELECT message.id, message.conversation_id, message.ordinal, message.role, message.action,
  length(message.content) AS content_code_points,
  length(CAST(message.content AS BLOB)) AS content_bytes,
  length(trim(message.content, char(9) || char(10) || char(11) || char(12) || char(13) || char(32))) > 0 AS content_nonblank,
  instr(message.content, char(0)) = 0 AND instr(message.content, char(13)) = 0 AS content_controls_valid,
  message.provider_id, message.model_id, message.citations_json, message.created_at
FROM messages message
JOIN conversations conversation ON conversation.id = message.conversation_id
WHERE conversation.book_id = ?
ORDER BY message.conversation_id, message.ordinal, message.id
LIMIT ?
"#,
    )
    .bind(book_id.to_string())
    .bind(limit)
    .fetch_all(&mut **transaction)
    .await?;
    if rows.len() > MAX_OVERVIEW_MESSAGES {
        return Err(database_error());
    }
    for row in rows {
        apply_message_row(&row, book_id, format, section_indexes, &mut conversations)?;
    }

    let mut completed_conversation_count = 0_u32;
    let mut completed_exchange_count = 0_u32;
    let mut citation_count = 0_u32;
    for conversation in conversations.values() {
        if conversation.message_count < 2
            || !conversation.message_count.is_multiple_of(2)
            || conversation.exchange_count != conversation.message_count / 2
        {
            return Err(database_error());
        }
        checked_add_assign(&mut completed_conversation_count, 1)?;
        checked_add_assign(&mut completed_exchange_count, conversation.exchange_count)?;
        checked_add_assign(&mut citation_count, conversation.citation_count)?;
        if let Some(section_index) = conversation.section_index {
            checked_add_assign(&mut sections[section_index].completed_conversation_count, 1)?;
            checked_add_assign(
                &mut sections[section_index].completed_exchange_count,
                conversation.exchange_count,
            )?;
        }
    }
    let history = &mut sources[LearningOverviewSource::HistorySummary.index()];
    history.item_count = completed_exchange_count;
    history.covered_section_count = to_u32(
        sections
            .iter()
            .filter(|section| section.completed_exchange_count > 0)
            .count(),
    )?;
    Ok(LearningOverviewActivity {
        user_note_count,
        completed_conversation_count,
        completed_exchange_count,
        citation_count,
    })
}

fn apply_message_row(
    row: &SqliteRow,
    book_id: Uuid,
    format: &BookFormat,
    section_indexes: &BTreeMap<Uuid, usize>,
    conversations: &mut BTreeMap<Uuid, ConversationState>,
) -> AppResult<()> {
    parse_uuid(row.try_get::<String, _>("id")?)?;
    let conversation_id = parse_uuid(row.try_get::<String, _>("conversation_id")?)?;
    let conversation = conversations
        .get_mut(&conversation_id)
        .ok_or_else(database_error)?;
    let ordinal = to_u32_i64(row.try_get::<i64, _>("ordinal")?)?;
    if ordinal != conversation.message_count
        || row.try_get::<i64, _>("content_nonblank")? != 1
        || row.try_get::<i64, _>("content_controls_valid")? != 1
    {
        return Err(database_error());
    }
    let role: String = row.try_get("role")?;
    let action: String = row.try_get("action")?;
    if !matches!(
        action.as_str(),
        "explain" | "example" | "derive" | "translate" | "ask" | "continue" | "overview"
    ) {
        return Err(database_error());
    }
    let content_code_points = row.try_get::<i64, _>("content_code_points")?;
    let content_bytes = row.try_get::<i64, _>("content_bytes")?;
    let provider_id = row.try_get::<Option<String>, _>("provider_id")?;
    let model_id = row.try_get::<Option<String>, _>("model_id")?;
    let citations_json = row.try_get::<Option<String>, _>("citations_json")?;
    match (ordinal % 2, role.as_str()) {
        (0, "user") => {
            if provider_id.is_some()
                || model_id.is_some()
                || citations_json.is_some()
                || !within_i64_limit(content_code_points, MAX_LEARNING_QUESTION_CODE_POINTS)
                || !within_i64_limit(content_bytes, MAX_LEARNING_QUESTION_BYTES)
            {
                return Err(database_error());
            }
            conversation.previous_action = Some(action);
        }
        (1, "assistant") => {
            parse_uuid(provider_id.ok_or_else(database_error)?)?;
            let model_id = model_id.ok_or_else(database_error)?;
            if model_id.trim() != model_id
                || model_id.is_empty()
                || model_id.chars().count() > MAX_LEARNING_MODEL_ID_CODE_POINTS
                || model_id.chars().any(char::is_control)
                || !within_i64_limit(content_code_points, MAX_LEARNING_ANSWER_CODE_POINTS)
                || !within_i64_limit(content_bytes, MAX_LEARNING_ANSWER_BYTES)
                || conversation.previous_action.as_deref() != Some(action.as_str())
            {
                return Err(database_error());
            }
            let citations_json = citations_json.ok_or_else(database_error)?;
            if citations_json.len() > MAX_LEARNING_CITATIONS_JSON_BYTES {
                return Err(database_error());
            }
            let citations: Vec<Citation> =
                serde_json::from_str(&citations_json).map_err(|_| database_error())?;
            if citations.len() > MAX_LEARNING_CITATIONS {
                return Err(database_error());
            }
            let mut citation_ids = BTreeSet::new();
            for citation in &citations {
                if citation.book_id != book_id
                    || !citation.quoteable
                    || !citation_ids.insert(citation.id.as_str())
                    || !locator_matches_format(&citation.locator, format)
                {
                    return Err(database_error());
                }
                match citation.source {
                    ContentSource::LocalText => {
                        if citation
                            .section_id
                            .is_none_or(|id| !section_indexes.contains_key(&id))
                        {
                            return Err(database_error());
                        }
                    }
                    ContentSource::AiTranscribed | ContentSource::UserCorrected => {
                        if citation.section_id.is_some() {
                            return Err(database_error());
                        }
                    }
                    ContentSource::AiDescription => return Err(database_error()),
                }
            }
            checked_add_assign(&mut conversation.citation_count, to_u32(citations.len())?)?;
            checked_add_assign(&mut conversation.exchange_count, 1)?;
            conversation.previous_action = None;
        }
        _ => return Err(database_error()),
    }
    let created_at = parse_database_timestamp(&row.try_get::<String, _>("created_at")?)?;
    if created_at < conversation.created_at
        || created_at > conversation.updated_at
        || conversation
            .previous_message_at
            .is_some_and(|previous| previous > created_at)
    {
        return Err(database_error());
    }
    conversation.previous_message_at = Some(created_at);
    checked_add_assign(&mut conversation.message_count, 1)
}

async fn record_query<O>(query_count: &mut u8, observer: &O) -> AppResult<()>
where
    O: OverviewReadObserver + ?Sized,
{
    *query_count = query_count.checked_add(1).ok_or_else(database_error)?;
    observer.after_query(*query_count).await;
    Ok(())
}

fn locator_matches_format_and_section(
    locator: &DocumentLocator,
    format: &BookFormat,
    section_id: Uuid,
) -> bool {
    match (locator, format) {
        (DocumentLocator::Pdf { .. }, BookFormat::Pdf) => true,
        (
            DocumentLocator::Epub {
                section_id: locator_section,
                ..
            },
            BookFormat::Epub,
        ) => *locator_section == section_id,
        (DocumentLocator::Docx { .. }, BookFormat::Docx) => true,
        _ => false,
    }
}

fn locator_matches_format(locator: &DocumentLocator, format: &BookFormat) -> bool {
    matches!(
        (locator, format),
        (DocumentLocator::Pdf { .. }, BookFormat::Pdf)
            | (DocumentLocator::Epub { .. }, BookFormat::Epub)
            | (DocumentLocator::Docx { .. }, BookFormat::Docx)
    )
}

fn parse_book_format(value: &str) -> AppResult<BookFormat> {
    match value {
        "pdf" => Ok(BookFormat::Pdf),
        "epub" => Ok(BookFormat::Epub),
        "docx" => Ok(BookFormat::Docx),
        _ => Err(database_error()),
    }
}

fn valid_section_title(value: &str) -> bool {
    !value.is_empty()
        && value.trim() == value
        && value.len() <= MAX_SECTION_TITLE_BYTES
        && value.chars().count() <= MAX_SECTION_TITLE_CODE_POINTS
        && !value
            .chars()
            .any(|character| character.is_control() && character != '\t')
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn sha256_text(value: &str) -> String {
    Sha256::digest(value.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn stable_correction_id(page_id: Uuid, block_id: Uuid, value_kind: &str) -> AppResult<Uuid> {
    if !matches!(value_kind, "text" | "latex") {
        return Err(database_error());
    }
    Ok(Uuid::new_v5(
        &page_id,
        format!("index-correction:{block_id}:{value_kind}").as_bytes(),
    ))
}

fn within_i64_limit(value: i64, maximum: usize) -> bool {
    value > 0 && usize::try_from(value).is_ok_and(|value| value <= maximum)
}

fn parse_uuid(value: impl AsRef<str>) -> AppResult<Uuid> {
    Uuid::parse_str(value.as_ref()).map_err(|_| database_error())
}

fn to_u32(value: usize) -> AppResult<u32> {
    u32::try_from(value).map_err(|_| database_error())
}

fn to_u32_i64(value: i64) -> AppResult<u32> {
    u32::try_from(value).map_err(|_| database_error())
}

fn checked_add_assign(target: &mut u32, value: u32) -> AppResult<()> {
    *target = target.checked_add(value).ok_or_else(database_error)?;
    Ok(())
}

fn database_error() -> AppError {
    AppError::new(AppErrorCode::DatabaseError)
}

#[cfg(test)]
#[path = "overview_test.rs"]
mod tests;
