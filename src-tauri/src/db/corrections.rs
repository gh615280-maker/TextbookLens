use std::collections::BTreeMap;

use chrono::Utc;
use sha2::{Digest, Sha256};
use sqlx::{Row, Sqlite, SqlitePool, Transaction, sqlite::SqliteRow};
use uuid::Uuid;

use crate::{
    domain::{
        ContentSource, DocumentLocator, IndexCorrectionConflictState, IndexCorrectionReviewDto,
        IndexCorrectionValueKind, NormalizedRect, stable_index_search_chunk_id,
    },
    errors::{AppError, AppErrorCode, AppResult},
};

use super::indexing::{
    database_contract_error, database_timestamp, parse_timestamp, parse_uuid, to_u32,
};

const MAX_TEXT_CODE_POINTS: usize = 65_536;
const MAX_TEXT_BYTES: usize = MAX_TEXT_CODE_POINTS * 4;
const MAX_LATEX_CODE_POINTS: usize = 16_384;
const MAX_LATEX_BYTES: usize = MAX_LATEX_CODE_POINTS * 4;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SaveIndexCorrection {
    pub book_id: Uuid,
    pub page_id: Uuid,
    pub target_block_id: Uuid,
    pub target_content_version: u32,
    pub value_kind: IndexCorrectionValueKind,
    pub original_value_sha256: String,
    pub corrected_value: String,
    /// Zero creates a correction. A positive value must exactly match the current revision.
    pub expected_revision: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CorrectionConflictDecision {
    Keep,
    Accept,
    Compare,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolveIndexCorrectionConflict {
    pub book_id: Uuid,
    pub page_id: Uuid,
    pub correction_id: Uuid,
    pub target_content_version: u32,
    pub current_value_sha256: Option<String>,
    pub expected_revision: u32,
    pub decision: CorrectionConflictDecision,
    pub compared_corrected_value: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeleteIndexCorrection {
    pub book_id: Uuid,
    pub page_id: Uuid,
    pub correction_id: Uuid,
    pub target_content_version: u32,
    pub current_value_sha256: Option<String>,
    pub expected_revision: u32,
}

pub async fn list_page_corrections(
    pool: &SqlitePool,
    page_id: Uuid,
) -> AppResult<Vec<IndexCorrectionReviewDto>> {
    let rows = sqlx::query(
        "SELECT id, target_block_id, region_x, region_y, region_width, region_height, value_kind, original_value, corrected_value, conflict_state, revision, updated_at FROM index_corrections WHERE page_id = ? ORDER BY created_at, id",
    )
    .bind(page_id.to_string())
    .fetch_all(pool)
    .await?;

    rows.iter().map(correction_from_row).collect()
}

pub async fn save_index_correction(
    pool: &SqlitePool,
    input: SaveIndexCorrection,
) -> AppResult<IndexCorrectionReviewDto> {
    validate_hash(&input.original_value_sha256)?;
    validate_corrected_value(input.value_kind, &input.corrected_value)?;
    let mut transaction = pool.begin().await?;
    let target = load_current_target(
        &mut transaction,
        input.book_id,
        input.page_id,
        input.target_block_id,
        input.target_content_version,
        input.value_kind,
    )
    .await?;
    let original_hash = sha256_text(&target.provider_value);
    if original_hash != input.original_value_sha256 {
        return Err(AppError::new(AppErrorCode::RequestConflict));
    }

    let timestamp = database_timestamp(Utc::now());
    let correction_id =
        stable_correction_id(input.page_id, input.target_block_id, input.value_kind);
    if input.expected_revision == 0 {
        let existing: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM index_corrections WHERE page_id = ? AND target_block_id = ? AND value_kind = ?",
        )
        .bind(input.page_id.to_string())
        .bind(input.target_block_id.to_string())
        .bind(value_kind_database(input.value_kind))
        .fetch_one(&mut *transaction)
        .await?;
        if existing != 0 {
            return Err(AppError::new(AppErrorCode::RequestConflict));
        }
        let (x, y, width, height) = rect_columns(target.bounds.as_ref());
        sqlx::query(
            "INSERT INTO index_corrections (id, book_id, page_id, target_block_id, target_content_version, region_x, region_y, region_width, region_height, value_kind, original_value_sha256, original_value, corrected_value, conflict_state, revision, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'active', 1, ?, ?)",
        )
        .bind(correction_id.to_string())
        .bind(input.book_id.to_string())
        .bind(input.page_id.to_string())
        .bind(input.target_block_id.to_string())
        .bind(i64::from(input.target_content_version))
        .bind(x)
        .bind(y)
        .bind(width)
        .bind(height)
        .bind(value_kind_database(input.value_kind))
        .bind(&original_hash)
        .bind(&target.provider_value)
        .bind(&input.corrected_value)
        .bind(&timestamp)
        .bind(&timestamp)
        .execute(&mut *transaction)
        .await?;
    } else {
        let result = sqlx::query(
            "UPDATE index_corrections SET corrected_value = ?, target_content_version = ?, revision = revision + 1, updated_at = ? WHERE id = ? AND book_id = ? AND page_id = ? AND target_block_id = ? AND value_kind = ? AND original_value_sha256 = ? AND conflict_state = 'active' AND revision = ? AND revision < 1000000",
        )
        .bind(&input.corrected_value)
        .bind(i64::from(input.target_content_version))
        .bind(&timestamp)
        .bind(correction_id.to_string())
        .bind(input.book_id.to_string())
        .bind(input.page_id.to_string())
        .bind(input.target_block_id.to_string())
        .bind(value_kind_database(input.value_kind))
        .bind(&original_hash)
        .bind(i64::from(input.expected_revision))
        .execute(&mut *transaction)
        .await?;
        require_one_row(result.rows_affected())?;
    }
    refresh_search_value(
        &mut transaction,
        &target,
        SearchOverlay::Corrected {
            correction_id,
            value: &input.corrected_value,
        },
        &timestamp,
    )
    .await?;
    transaction.commit().await?;
    get_correction(pool, correction_id, input.book_id, input.page_id).await
}

pub async fn resolve_index_correction_conflict(
    pool: &SqlitePool,
    input: ResolveIndexCorrectionConflict,
) -> AppResult<Option<IndexCorrectionReviewDto>> {
    if input.expected_revision == 0 {
        return Err(AppError::new(AppErrorCode::InvalidInput));
    }
    if let Some(hash) = &input.current_value_sha256 {
        validate_hash(hash)?;
    }
    if input.decision == CorrectionConflictDecision::Compare {
        input
            .compared_corrected_value
            .as_deref()
            .ok_or_else(|| AppError::new(AppErrorCode::InvalidInput))?;
    } else if input.compared_corrected_value.is_some() {
        return Err(AppError::new(AppErrorCode::InvalidInput));
    }

    let mut transaction = pool.begin().await?;
    let correction = load_owned_correction(
        &mut transaction,
        input.book_id,
        input.page_id,
        input.correction_id,
        input.expected_revision,
    )
    .await?;
    if correction.conflict_state != IndexCorrectionConflictState::Conflict {
        return Err(AppError::new(AppErrorCode::RequestConflict));
    }
    if let Some(value) = input.compared_corrected_value.as_deref() {
        validate_corrected_value(correction.value_kind, value)?;
    }
    let target =
        load_target_for_correction(&mut transaction, &correction, input.target_content_version)
            .await?;
    verify_current_hash(target.as_ref(), input.current_value_sha256.as_deref())?;
    let timestamp = database_timestamp(Utc::now());

    match input.decision {
        CorrectionConflictDecision::Accept => {
            delete_owned_correction(&mut transaction, &correction).await?;
            if let Some(target) = &target {
                refresh_search_value(
                    &mut transaction,
                    target,
                    SearchOverlay::Provider,
                    &timestamp,
                )
                .await?;
            } else {
                remove_unreferenced_stale_block(
                    &mut transaction,
                    correction.page_id,
                    correction.target_block_id,
                    input.target_content_version,
                )
                .await?;
            }
            transaction.commit().await?;
            Ok(None)
        }
        CorrectionConflictDecision::Keep | CorrectionConflictDecision::Compare => {
            let target = target.ok_or_else(|| AppError::new(AppErrorCode::RequestConflict))?;
            let corrected_value = input
                .compared_corrected_value
                .as_deref()
                .unwrap_or(&correction.corrected_value);
            let current_hash = sha256_text(&target.provider_value);
            let result = sqlx::query(
                "UPDATE index_corrections SET target_content_version = ?, original_value_sha256 = ?, original_value = ?, corrected_value = ?, conflict_state = 'active', revision = revision + 1, updated_at = ? WHERE id = ? AND book_id = ? AND page_id = ? AND revision = ? AND conflict_state = 'conflict' AND revision < 1000000",
            )
            .bind(i64::from(input.target_content_version))
            .bind(&current_hash)
            .bind(&target.provider_value)
            .bind(corrected_value)
            .bind(&timestamp)
            .bind(correction.id.to_string())
            .bind(correction.book_id.to_string())
            .bind(correction.page_id.to_string())
            .bind(i64::from(input.expected_revision))
            .execute(&mut *transaction)
            .await?;
            require_one_row(result.rows_affected())?;
            refresh_search_value(
                &mut transaction,
                &target,
                SearchOverlay::Corrected {
                    correction_id: correction.id,
                    value: corrected_value,
                },
                &timestamp,
            )
            .await?;
            transaction.commit().await?;
            get_correction(pool, correction.id, correction.book_id, correction.page_id)
                .await
                .map(Some)
        }
    }
}

pub async fn delete_index_correction(
    pool: &SqlitePool,
    input: DeleteIndexCorrection,
) -> AppResult<()> {
    if input.expected_revision == 0 {
        return Err(AppError::new(AppErrorCode::InvalidInput));
    }
    if let Some(hash) = &input.current_value_sha256 {
        validate_hash(hash)?;
    }
    let mut transaction = pool.begin().await?;
    let correction = load_owned_correction(
        &mut transaction,
        input.book_id,
        input.page_id,
        input.correction_id,
        input.expected_revision,
    )
    .await?;
    let target =
        load_target_for_correction(&mut transaction, &correction, input.target_content_version)
            .await?;
    verify_current_hash(target.as_ref(), input.current_value_sha256.as_deref())?;
    delete_owned_correction(&mut transaction, &correction).await?;
    let timestamp = database_timestamp(Utc::now());
    if let Some(target) = &target {
        refresh_search_value(
            &mut transaction,
            target,
            SearchOverlay::Provider,
            &timestamp,
        )
        .await?;
    } else {
        remove_unreferenced_stale_block(
            &mut transaction,
            correction.page_id,
            correction.target_block_id,
            input.target_content_version,
        )
        .await?;
    }
    transaction.commit().await?;
    Ok(())
}

#[derive(Clone, Debug)]
struct CurrentTarget {
    book_id: Uuid,
    page_id: Uuid,
    block_id: Uuid,
    page_number: u32,
    content_version: u32,
    block_ordinal: u32,
    value_kind: IndexCorrectionValueKind,
    provider_value: String,
    bounds: Option<NormalizedRect>,
}

#[derive(Clone, Debug)]
struct OwnedCorrection {
    id: Uuid,
    book_id: Uuid,
    page_id: Uuid,
    target_block_id: Uuid,
    value_kind: IndexCorrectionValueKind,
    corrected_value: String,
    conflict_state: IndexCorrectionConflictState,
    revision: u32,
}

async fn load_current_target(
    transaction: &mut Transaction<'_, Sqlite>,
    book_id: Uuid,
    page_id: Uuid,
    block_id: Uuid,
    content_version: u32,
    value_kind: IndexCorrectionValueKind,
) -> AppResult<CurrentTarget> {
    let page =
        sqlx::query("SELECT book_id, page_number, content_version FROM index_pages WHERE id = ?")
            .bind(page_id.to_string())
            .fetch_optional(&mut **transaction)
            .await?
            .ok_or_else(|| AppError::new(AppErrorCode::NotFound))?;
    let actual_book_id = parse_uuid(page.try_get::<String, _>("book_id")?)?;
    let actual_version = to_u32(page.try_get::<i64, _>("content_version")?)?;
    if actual_book_id != book_id || actual_version != content_version || content_version == 0 {
        return Err(AppError::new(AppErrorCode::RequestConflict));
    }
    let block = sqlx::query(
        "SELECT page_id, book_id, ordinal, plain_text, latex, bounds_x, bounds_y, bounds_width, bounds_height, content_version FROM index_page_blocks WHERE id = ?",
    )
    .bind(block_id.to_string())
    .fetch_optional(&mut **transaction)
    .await?
    .ok_or_else(|| AppError::new(AppErrorCode::NotFound))?;
    if parse_uuid(block.try_get::<String, _>("page_id")?)? != page_id
        || parse_uuid(block.try_get::<String, _>("book_id")?)? != book_id
        || to_u32(block.try_get::<i64, _>("content_version")?)? != content_version
    {
        return Err(AppError::new(AppErrorCode::RequestConflict));
    }
    let provider_value = target_value_from_row(&block, value_kind)?;
    Ok(CurrentTarget {
        book_id,
        page_id,
        block_id,
        page_number: to_u32(page.try_get::<i64, _>("page_number")?)?,
        content_version,
        block_ordinal: to_u32(block.try_get::<i64, _>("ordinal")?)?,
        value_kind,
        provider_value,
        bounds: bounds_from_row(&block)?,
    })
}

async fn load_owned_correction(
    transaction: &mut Transaction<'_, Sqlite>,
    book_id: Uuid,
    page_id: Uuid,
    correction_id: Uuid,
    expected_revision: u32,
) -> AppResult<OwnedCorrection> {
    let row = sqlx::query(
        "SELECT id, book_id, page_id, target_block_id, value_kind, corrected_value, conflict_state, revision FROM index_corrections WHERE id = ?",
    )
    .bind(correction_id.to_string())
    .fetch_optional(&mut **transaction)
    .await?
    .ok_or_else(|| AppError::new(AppErrorCode::NotFound))?;
    let correction = OwnedCorrection {
        id: parse_uuid(row.try_get::<String, _>("id")?)?,
        book_id: parse_uuid(row.try_get::<String, _>("book_id")?)?,
        page_id: parse_uuid(row.try_get::<String, _>("page_id")?)?,
        target_block_id: parse_uuid(row.try_get::<String, _>("target_block_id")?)?,
        value_kind: parse_value_kind(&row.try_get::<String, _>("value_kind")?)?,
        corrected_value: row.try_get("corrected_value")?,
        conflict_state: parse_conflict_state(&row.try_get::<String, _>("conflict_state")?)?,
        revision: to_u32(row.try_get::<i64, _>("revision")?)?,
    };
    if correction.book_id != book_id
        || correction.page_id != page_id
        || correction.revision != expected_revision
    {
        return Err(AppError::new(AppErrorCode::RequestConflict));
    }
    Ok(correction)
}

async fn load_target_for_correction(
    transaction: &mut Transaction<'_, Sqlite>,
    correction: &OwnedCorrection,
    current_content_version: u32,
) -> AppResult<Option<CurrentTarget>> {
    let page =
        sqlx::query("SELECT book_id, page_number, content_version FROM index_pages WHERE id = ?")
            .bind(correction.page_id.to_string())
            .fetch_optional(&mut **transaction)
            .await?
            .ok_or_else(|| AppError::new(AppErrorCode::NotFound))?;
    let page_book_id = parse_uuid(page.try_get::<String, _>("book_id")?)?;
    let page_version = to_u32(page.try_get::<i64, _>("content_version")?)?;
    if page_book_id != correction.book_id
        || page_version != current_content_version
        || current_content_version == 0
    {
        return Err(AppError::new(AppErrorCode::RequestConflict));
    }
    let block = sqlx::query(
        "SELECT page_id, book_id, ordinal, plain_text, latex, bounds_x, bounds_y, bounds_width, bounds_height, content_version FROM index_page_blocks WHERE id = ?",
    )
    .bind(correction.target_block_id.to_string())
    .fetch_optional(&mut **transaction)
    .await?;
    let Some(block) = block else {
        return Ok(None);
    };
    if parse_uuid(block.try_get::<String, _>("page_id")?)? != correction.page_id
        || parse_uuid(block.try_get::<String, _>("book_id")?)? != correction.book_id
    {
        return Err(AppError::new(AppErrorCode::RequestConflict));
    }
    if to_u32(block.try_get::<i64, _>("content_version")?)? != current_content_version {
        return Ok(None);
    }
    Ok(Some(CurrentTarget {
        book_id: correction.book_id,
        page_id: correction.page_id,
        block_id: correction.target_block_id,
        page_number: to_u32(page.try_get::<i64, _>("page_number")?)?,
        content_version: current_content_version,
        block_ordinal: to_u32(block.try_get::<i64, _>("ordinal")?)?,
        value_kind: correction.value_kind,
        provider_value: target_value_from_row(&block, correction.value_kind)?,
        bounds: bounds_from_row(&block)?,
    }))
}

async fn delete_owned_correction(
    transaction: &mut Transaction<'_, Sqlite>,
    correction: &OwnedCorrection,
) -> AppResult<()> {
    let result = sqlx::query(
        "DELETE FROM index_corrections WHERE id = ? AND book_id = ? AND page_id = ? AND revision = ?",
    )
    .bind(correction.id.to_string())
    .bind(correction.book_id.to_string())
    .bind(correction.page_id.to_string())
    .bind(i64::from(correction.revision))
    .execute(&mut **transaction)
    .await?;
    require_one_row(result.rows_affected())
}

enum SearchOverlay<'a> {
    Provider,
    Corrected { correction_id: Uuid, value: &'a str },
}

async fn refresh_search_value(
    transaction: &mut Transaction<'_, Sqlite>,
    target: &CurrentTarget,
    overlay: SearchOverlay<'_>,
    timestamp: &str,
) -> AppResult<()> {
    let ordinal = target_chunk_ordinal(target.block_ordinal, target.value_kind)?;
    sqlx::query(
        "DELETE FROM index_search_chunks WHERE page_id = ? AND block_id = ? AND content_version = ? AND ordinal = ? AND source IN ('ai_transcribed', 'user_corrected')",
    )
    .bind(target.page_id.to_string())
    .bind(target.block_id.to_string())
    .bind(i64::from(target.content_version))
    .bind(i64::from(ordinal))
    .execute(&mut **transaction)
    .await?;
    let (source, correction_id, value) = match overlay {
        SearchOverlay::Provider => (
            ContentSource::AiTranscribed,
            None,
            target.provider_value.as_str(),
        ),
        SearchOverlay::Corrected {
            correction_id,
            value,
        } => (ContentSource::UserCorrected, Some(correction_id), value),
    };
    let locator_json = locator_json(target.page_number, target.bounds.as_ref())?;
    let chunk_id = stable_index_search_chunk_id(target.block_id, source, ordinal);
    let token_estimate = u32::try_from(value.chars().count().div_ceil(4).max(1))
        .map_err(|_| database_contract_error())?;
    sqlx::query(
        "INSERT INTO index_search_chunks (id, book_id, page_id, block_id, correction_id, ordinal, source, text, locator_json, token_estimate, content_version, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(chunk_id.to_string())
    .bind(target.book_id.to_string())
    .bind(target.page_id.to_string())
    .bind(target.block_id.to_string())
    .bind(correction_id.map(|id| id.to_string()))
    .bind(i64::from(ordinal))
    .bind(source.as_str())
    .bind(value)
    .bind(locator_json)
    .bind(i64::from(token_estimate))
    .bind(i64::from(target.content_version))
    .bind(timestamp)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

async fn remove_unreferenced_stale_block(
    transaction: &mut Transaction<'_, Sqlite>,
    page_id: Uuid,
    block_id: Uuid,
    current_content_version: u32,
) -> AppResult<()> {
    let remaining: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM index_corrections WHERE page_id = ? AND target_block_id = ?",
    )
    .bind(page_id.to_string())
    .bind(block_id.to_string())
    .fetch_one(&mut **transaction)
    .await?;
    if remaining == 0 {
        sqlx::query(
            "DELETE FROM index_page_blocks WHERE id = ? AND page_id = ? AND content_version != ?",
        )
        .bind(block_id.to_string())
        .bind(page_id.to_string())
        .bind(i64::from(current_content_version))
        .execute(&mut **transaction)
        .await?;
    }
    Ok(())
}

fn verify_current_hash(target: Option<&CurrentTarget>, supplied: Option<&str>) -> AppResult<()> {
    match (target, supplied) {
        (Some(target), Some(supplied)) if sha256_text(&target.provider_value) == supplied => Ok(()),
        (None, None) => Ok(()),
        _ => Err(AppError::new(AppErrorCode::RequestConflict)),
    }
}

async fn get_correction(
    pool: &SqlitePool,
    correction_id: Uuid,
    book_id: Uuid,
    page_id: Uuid,
) -> AppResult<IndexCorrectionReviewDto> {
    let row = sqlx::query(
        "SELECT id, target_block_id, region_x, region_y, region_width, region_height, value_kind, original_value, corrected_value, conflict_state, revision, updated_at FROM index_corrections WHERE id = ? AND book_id = ? AND page_id = ?",
    )
    .bind(correction_id.to_string())
    .bind(book_id.to_string())
    .bind(page_id.to_string())
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::new(AppErrorCode::NotFound))?;
    correction_from_row(&row)
}

fn correction_from_row(row: &SqliteRow) -> AppResult<IndexCorrectionReviewDto> {
    Ok(IndexCorrectionReviewDto {
        id: parse_uuid(row.try_get::<String, _>("id")?)?,
        target_block_id: parse_uuid(row.try_get::<String, _>("target_block_id")?)?,
        region: region_from_row(row)?,
        value_kind: parse_value_kind(&row.try_get::<String, _>("value_kind")?)?,
        original_value: row.try_get("original_value")?,
        corrected_value: row.try_get("corrected_value")?,
        conflict_state: parse_conflict_state(&row.try_get::<String, _>("conflict_state")?)?,
        revision: to_u32(row.try_get::<i64, _>("revision")?)?,
        updated_at: parse_timestamp(&row.try_get::<String, _>("updated_at")?)?,
    })
}

fn region_from_row(row: &SqliteRow) -> AppResult<Option<NormalizedRect>> {
    match (
        row.try_get::<Option<f64>, _>("region_x")?,
        row.try_get::<Option<f64>, _>("region_y")?,
        row.try_get::<Option<f64>, _>("region_width")?,
        row.try_get::<Option<f64>, _>("region_height")?,
    ) {
        (None, None, None, None) => Ok(None),
        (Some(x), Some(y), Some(width), Some(height)) => NormalizedRect::new(x, y, width, height)
            .map(Some)
            .map_err(|_| database_contract_error()),
        _ => Err(database_contract_error()),
    }
}

fn target_value_from_row(row: &SqliteRow, kind: IndexCorrectionValueKind) -> AppResult<String> {
    let value = match kind {
        IndexCorrectionValueKind::Text => row.try_get::<Option<String>, _>("plain_text")?,
        IndexCorrectionValueKind::Latex => row.try_get::<Option<String>, _>("latex")?,
    };
    value
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| AppError::new(AppErrorCode::RequestConflict))
}

fn bounds_from_row(row: &SqliteRow) -> AppResult<Option<NormalizedRect>> {
    match (
        row.try_get::<Option<f64>, _>("bounds_x")?,
        row.try_get::<Option<f64>, _>("bounds_y")?,
        row.try_get::<Option<f64>, _>("bounds_width")?,
        row.try_get::<Option<f64>, _>("bounds_height")?,
    ) {
        (None, None, None, None) => Ok(None),
        (Some(x), Some(y), Some(width), Some(height)) => NormalizedRect::new(x, y, width, height)
            .map(Some)
            .map_err(|_| database_contract_error()),
        _ => Err(database_contract_error()),
    }
}

fn locator_json(page_number: u32, bounds: Option<&NormalizedRect>) -> AppResult<String> {
    let rects = bounds.map(|bounds| BTreeMap::from([(page_number, vec![bounds.clone()])]));
    let locator = DocumentLocator::pdf(page_number, page_number, rects)
        .map_err(|_| database_contract_error())?;
    serde_json::to_string(&locator).map_err(AppError::database)
}

fn target_chunk_ordinal(block_ordinal: u32, kind: IndexCorrectionValueKind) -> AppResult<u32> {
    let offset = match kind {
        IndexCorrectionValueKind::Text => 0,
        IndexCorrectionValueKind::Latex => 1,
    };
    block_ordinal
        .checked_mul(4)
        .and_then(|base| base.checked_add(offset))
        .filter(|ordinal| *ordinal <= 100_000)
        .ok_or_else(|| AppError::new(AppErrorCode::InvalidInput))
}

fn stable_correction_id(page_id: Uuid, block_id: Uuid, kind: IndexCorrectionValueKind) -> Uuid {
    Uuid::new_v5(
        &page_id,
        format!("index-correction:{block_id}:{}", value_kind_database(kind)).as_bytes(),
    )
}

fn validate_corrected_value(kind: IndexCorrectionValueKind, value: &str) -> AppResult<()> {
    let (max_code_points, max_bytes) = match kind {
        IndexCorrectionValueKind::Text => (MAX_TEXT_CODE_POINTS, MAX_TEXT_BYTES),
        IndexCorrectionValueKind::Latex => (MAX_LATEX_CODE_POINTS, MAX_LATEX_BYTES),
    };
    if value.trim().is_empty()
        || value.len() > max_bytes
        || value.chars().count() > max_code_points
        || value.contains(['\0', '\r'])
        || value
            .chars()
            .any(|character| character.is_control() && !matches!(character, '\n' | '\t'))
    {
        Err(AppError::new(AppErrorCode::InvalidInput))
    } else {
        Ok(())
    }
}

fn validate_hash(value: &str) -> AppResult<()> {
    if value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(())
    } else {
        Err(AppError::new(AppErrorCode::InvalidInput))
    }
}

pub fn correction_value_sha256(value: &str) -> String {
    sha256_text(value)
}

fn sha256_text(value: &str) -> String {
    Sha256::digest(value.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn parse_value_kind(value: &str) -> AppResult<IndexCorrectionValueKind> {
    IndexCorrectionValueKind::from_database(value).ok_or_else(database_contract_error)
}

fn parse_conflict_state(value: &str) -> AppResult<IndexCorrectionConflictState> {
    IndexCorrectionConflictState::from_database(value).ok_or_else(database_contract_error)
}

const fn value_kind_database(kind: IndexCorrectionValueKind) -> &'static str {
    match kind {
        IndexCorrectionValueKind::Text => "text",
        IndexCorrectionValueKind::Latex => "latex",
    }
}

fn rect_columns(
    bounds: Option<&NormalizedRect>,
) -> (Option<f64>, Option<f64>, Option<f64>, Option<f64>) {
    bounds.map_or((None, None, None, None), |bounds| {
        (
            Some(bounds.x),
            Some(bounds.y),
            Some(bounds.width),
            Some(bounds.height),
        )
    })
}

fn require_one_row(rows_affected: u64) -> AppResult<()> {
    if rows_affected == 1 {
        Ok(())
    } else {
        Err(AppError::new(AppErrorCode::RequestConflict))
    }
}

#[cfg(test)]
#[path = "corrections_test.rs"]
mod tests;
