use std::{collections::BTreeMap, sync::Arc};

use chrono::Utc;
use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::{Row, SqliteConnection, SqlitePool};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{
    db::indexing::database_timestamp,
    domain::{
        ContentSource, DocumentLocator, IndexCorrectionConflictState, IndexCorrectionValueKind,
        IndexPageStatus, NormalizedRect, stable_index_page_block_id, stable_index_search_chunk_id,
    },
    errors::{AppError, AppErrorCode, AppResult},
};

use super::validator::{ValidatedBlock, ValidatedPage};

const PROVENANCE_VERSION: i64 = 1;
const CHUNKS_PER_BLOCK: u32 = 4;
const TEXT_CHUNK_OFFSET: u32 = 0;
const LATEX_CHUNK_OFFSET: u32 = 1;
const TABLE_CHUNK_OFFSET: u32 = 2;
const DESCRIPTION_CHUNK_OFFSET: u32 = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommitBarrier {
    BeforeTransaction,
    BeforeStatusAndCommit,
    AfterCommit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommitFault {
    BlockInsert,
    SearchInsert,
    StatusUpdate,
}

#[derive(Clone, Default)]
pub struct CommitControl {
    fault: Option<CommitFault>,
    barrier: Option<Arc<dyn Fn(CommitBarrier) + Send + Sync>>,
}

impl CommitControl {
    pub fn with_fault(fault: CommitFault) -> Self {
        Self {
            fault: Some(fault),
            barrier: None,
        }
    }

    pub fn with_barrier(barrier: impl Fn(CommitBarrier) + Send + Sync + 'static) -> Self {
        Self {
            fault: None,
            barrier: Some(Arc::new(barrier)),
        }
    }

    fn notify(&self, barrier: CommitBarrier) {
        if let Some(callback) = &self.barrier {
            callback(barrier);
        }
    }

    fn inject(&self, fault: CommitFault) -> AppResult<()> {
        if self.fault == Some(fault) {
            Err(AppError::database("simulated local page commit fault"))
        } else {
            Ok(())
        }
    }
}

pub struct PageCommitRequest<'a> {
    pub page_id: Uuid,
    pub attempt_id: Uuid,
    pub page: &'a ValidatedPage,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PageCommitOutcome {
    pub status: IndexPageStatus,
    pub content_version: u32,
    pub content_sha256: String,
}

#[derive(Clone, Copy)]
struct BlockOwner {
    page_id: Uuid,
    run_id: Uuid,
    book_id: Uuid,
}

pub async fn commit_validated_page(
    pool: &SqlitePool,
    request: PageCommitRequest<'_>,
    cancellation: &CancellationToken,
) -> AppResult<PageCommitOutcome> {
    commit_validated_page_with_control(pool, request, cancellation, &CommitControl::default()).await
}

pub async fn commit_validated_page_with_control(
    pool: &SqlitePool,
    request: PageCommitRequest<'_>,
    cancellation: &CancellationToken,
    control: &CommitControl,
) -> AppResult<PageCommitOutcome> {
    control.notify(CommitBarrier::BeforeTransaction);
    require_not_cancelled(cancellation)?;

    let mut transaction = pool.begin().await?;
    let owner = sqlx::query(
        "SELECT p.book_id, p.run_id, p.page_number, p.content_version, p.status, p.attempt_id, r.analysis_schema_version, r.status AS run_status FROM index_pages p JOIN index_runs r ON r.id = p.run_id AND r.book_id = p.book_id WHERE p.id = ?",
    )
    .bind(request.page_id.to_string())
    .fetch_optional(&mut *transaction)
    .await?
    .ok_or_else(|| AppError::new(AppErrorCode::NotFound))?;
    let book_id = parse_uuid(owner.try_get::<String, _>("book_id")?)?;
    let run_id = parse_uuid(owner.try_get::<String, _>("run_id")?)?;
    let page_number = u32::try_from(owner.try_get::<i64, _>("page_number")?)
        .map_err(|_| database_contract_error())?;
    let current_version = u32::try_from(owner.try_get::<i64, _>("content_version")?)
        .map_err(|_| database_contract_error())?;
    let attempt_id = owner
        .try_get::<Option<String>, _>("attempt_id")?
        .map(parse_uuid)
        .transpose()?;
    if page_number != request.page.page_number
        || owner.try_get::<String, _>("status")? != IndexPageStatus::Parsing.as_str()
        || attempt_id != Some(request.attempt_id)
        || !matches!(
            owner.try_get::<String, _>("run_status")?.as_str(),
            "running" | "paused"
        )
    {
        return Err(AppError::new(AppErrorCode::RequestConflict));
    }
    let next_version = current_version
        .checked_add(1)
        .filter(|version| *version <= 1_000_000)
        .ok_or_else(database_contract_error)?;
    let schema_version: String = owner.try_get("analysis_schema_version")?;
    let timestamp = database_timestamp(Utc::now());

    let validating = sqlx::query(
        "UPDATE index_pages SET status = 'validating', updated_at = ? WHERE id = ? AND status = 'parsing' AND attempt_id = ? AND EXISTS (SELECT 1 FROM index_runs r WHERE r.id = index_pages.run_id AND r.status IN ('running', 'paused'))",
    )
    .bind(&timestamp)
    .bind(request.page_id.to_string())
    .bind(request.attempt_id.to_string())
    .execute(&mut *transaction)
    .await?;
    require_one_row(validating.rows_affected())?;

    sqlx::query("DELETE FROM index_search_chunks WHERE page_id = ?")
        .bind(request.page_id.to_string())
        .execute(&mut *transaction)
        .await?;

    let mut current_block_ids = Vec::with_capacity(request.page.blocks.len());
    for block in &request.page.blocks {
        let block_id = stable_index_page_block_id(
            book_id,
            page_number,
            run_id,
            &schema_version,
            block.ordinal,
        );
        current_block_ids.push(block_id);
        upsert_block(
            &mut transaction,
            BlockOwner {
                page_id: request.page_id,
                run_id,
                book_id,
            },
            block_id,
            block,
            next_version,
            &timestamp,
        )
        .await?;
        reconcile_existing_corrections(
            &mut transaction,
            request.page_id,
            book_id,
            block_id,
            block,
            next_version,
            &timestamp,
        )
        .await?;
    }
    control.inject(CommitFault::BlockInsert)?;

    retain_only_current_or_corrected_blocks(
        &mut transaction,
        request.page_id,
        &current_block_ids,
        &timestamp,
    )
    .await?;

    for (block, block_id) in request.page.blocks.iter().zip(current_block_ids.iter()) {
        insert_block_search_rows(
            &mut transaction,
            SearchOwner {
                page_id: request.page_id,
                book_id,
                block_id: *block_id,
                page_number,
                content_version: next_version,
            },
            block,
            &timestamp,
        )
        .await?;
    }
    control.inject(CommitFault::SearchInsert)?;

    control.notify(CommitBarrier::BeforeStatusAndCommit);
    require_not_cancelled(cancellation)?;
    let content_sha256 = sha256_serialized(request.page)?;
    let status = if request.page.review_reason.is_some() {
        IndexPageStatus::NeedsReview
    } else {
        IndexPageStatus::Indexed
    };
    let final_status = sqlx::query(
        "UPDATE index_pages SET status = ?, content_sha256 = ?, content_version = ?, review_reason_code = ?, safe_error_code = NULL, safe_error_message = NULL, retryable = 0, updated_at = ? WHERE id = ? AND status = 'validating' AND attempt_id = ? AND EXISTS (SELECT 1 FROM index_runs r WHERE r.id = index_pages.run_id AND r.status IN ('running', 'paused'))",
    )
    .bind(status.as_str())
    .bind(&content_sha256)
    .bind(i64::from(next_version))
    .bind(request.page.review_reason.map(|reason| reason.as_str()))
    .bind(&timestamp)
    .bind(request.page_id.to_string())
    .bind(request.attempt_id.to_string())
    .execute(&mut *transaction)
    .await?;
    require_one_row(final_status.rows_affected())?;
    control.inject(CommitFault::StatusUpdate)?;

    transaction.commit().await?;
    control.notify(CommitBarrier::AfterCommit);
    Ok(PageCommitOutcome {
        status,
        content_version: next_version,
        content_sha256,
    })
}

async fn upsert_block(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    owner: BlockOwner,
    block_id: Uuid,
    block: &ValidatedBlock,
    content_version: u32,
    timestamp: &str,
) -> AppResult<()> {
    let table_json = block
        .table_cells
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .map_err(AppError::database)?;
    let value_sha256 = sha256_serialized(block)?;
    let (x, y, width, height) = rect_columns(block.bounds.as_ref());
    let result = sqlx::query(
        "INSERT INTO index_page_blocks (id, page_id, run_id, book_id, ordinal, kind, plain_text, latex, table_json, visual_description, bounds_x, bounds_y, bounds_width, bounds_height, source, provenance_version, content_version, value_sha256, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT(id) DO UPDATE SET ordinal = excluded.ordinal, kind = excluded.kind, plain_text = excluded.plain_text, latex = excluded.latex, table_json = excluded.table_json, visual_description = excluded.visual_description, bounds_x = excluded.bounds_x, bounds_y = excluded.bounds_y, bounds_width = excluded.bounds_width, bounds_height = excluded.bounds_height, source = excluded.source, provenance_version = excluded.provenance_version, content_version = excluded.content_version, value_sha256 = excluded.value_sha256, created_at = excluded.created_at WHERE index_page_blocks.page_id = excluded.page_id AND index_page_blocks.run_id = excluded.run_id AND index_page_blocks.book_id = excluded.book_id",
    )
    .bind(block_id.to_string())
    .bind(owner.page_id.to_string())
    .bind(owner.run_id.to_string())
    .bind(owner.book_id.to_string())
    .bind(i64::from(block.ordinal))
    .bind(block.kind.as_str())
    .bind(&block.plain_text)
    .bind(&block.latex)
    .bind(table_json)
    .bind(&block.visual_description)
    .bind(x)
    .bind(y)
    .bind(width)
    .bind(height)
    .bind(block.source.as_str())
    .bind(PROVENANCE_VERSION)
    .bind(i64::from(content_version))
    .bind(value_sha256)
    .bind(timestamp)
    .execute(&mut **transaction)
    .await?;
    require_one_row(result.rows_affected())
}

async fn reconcile_existing_corrections(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    page_id: Uuid,
    book_id: Uuid,
    block_id: Uuid,
    block: &ValidatedBlock,
    content_version: u32,
    timestamp: &str,
) -> AppResult<()> {
    let corrections = sqlx::query(
        "SELECT id, value_kind, original_value_sha256, conflict_state FROM index_corrections WHERE page_id = ? AND book_id = ? AND target_block_id = ? ORDER BY value_kind",
    )
    .bind(page_id.to_string())
    .bind(book_id.to_string())
    .bind(block_id.to_string())
    .fetch_all(&mut **transaction)
    .await?;
    for correction in corrections {
        let value_kind = parse_value_kind(&correction.try_get::<String, _>("value_kind")?)?;
        let current_hash = correction_target_value(block, value_kind).map(sha256_text);
        let original_hash: String = correction.try_get("original_value_sha256")?;
        let existing_state =
            parse_conflict_state(&correction.try_get::<String, _>("conflict_state")?)?;
        let next_state = if existing_state == IndexCorrectionConflictState::Active
            && current_hash.as_deref() == Some(original_hash.as_str())
        {
            IndexCorrectionConflictState::Active
        } else {
            IndexCorrectionConflictState::Conflict
        };
        let result = sqlx::query(
            "UPDATE index_corrections SET target_content_version = ?, conflict_state = ?, revision = revision + 1, updated_at = ? WHERE id = ? AND page_id = ? AND book_id = ? AND revision < 1000000",
        )
        .bind(i64::from(content_version))
        .bind(conflict_state_database(next_state))
        .bind(timestamp)
        .bind(correction.try_get::<String, _>("id")?)
        .bind(page_id.to_string())
        .bind(book_id.to_string())
        .execute(&mut **transaction)
        .await?;
        require_one_row(result.rows_affected())?;
    }
    Ok(())
}

async fn retain_only_current_or_corrected_blocks(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    page_id: Uuid,
    current_block_ids: &[Uuid],
    timestamp: &str,
) -> AppResult<()> {
    let rows = sqlx::query("SELECT id FROM index_page_blocks WHERE page_id = ? ORDER BY ordinal")
        .bind(page_id.to_string())
        .fetch_all(&mut **transaction)
        .await?;
    for row in rows {
        let block_id = parse_uuid(row.try_get::<String, _>("id")?)?;
        if current_block_ids.contains(&block_id) {
            continue;
        }
        let correction_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM index_corrections WHERE page_id = ? AND target_block_id = ?",
        )
        .bind(page_id.to_string())
        .bind(block_id.to_string())
        .fetch_one(&mut **transaction)
        .await?;
        if correction_count == 0 {
            sqlx::query("DELETE FROM index_page_blocks WHERE id = ? AND page_id = ?")
                .bind(block_id.to_string())
                .bind(page_id.to_string())
                .execute(&mut **transaction)
                .await?;
        } else {
            sqlx::query(
                "UPDATE index_corrections SET conflict_state = 'conflict', revision = revision + 1, updated_at = ? WHERE page_id = ? AND target_block_id = ? AND revision < 1000000",
            )
            .bind(timestamp)
            .bind(page_id.to_string())
            .bind(block_id.to_string())
            .execute(&mut **transaction)
            .await?;
        }
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct SearchOwner {
    page_id: Uuid,
    book_id: Uuid,
    block_id: Uuid,
    page_number: u32,
    content_version: u32,
}

async fn insert_block_search_rows(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    owner: SearchOwner,
    block: &ValidatedBlock,
    timestamp: &str,
) -> AppResult<()> {
    let corrections = sqlx::query(
        "SELECT id, value_kind, corrected_value, conflict_state FROM index_corrections WHERE page_id = ? AND book_id = ? AND target_block_id = ? ORDER BY value_kind",
    )
    .bind(owner.page_id.to_string())
    .bind(owner.book_id.to_string())
    .bind(owner.block_id.to_string())
    .fetch_all(&mut **transaction)
    .await?;
    let mut correction_by_kind = BTreeMap::new();
    for correction in corrections {
        let kind = parse_value_kind(&correction.try_get::<String, _>("value_kind")?)?;
        correction_by_kind.insert(
            correction_kind_database(kind),
            CorrectionSearchState {
                id: parse_uuid(correction.try_get::<String, _>("id")?)?,
                corrected_value: correction.try_get("corrected_value")?,
                conflict_state: parse_conflict_state(
                    &correction.try_get::<String, _>("conflict_state")?,
                )?,
            },
        );
    }

    if let Some(text) = block.plain_text.as_deref() {
        insert_correctable_search_value(
            transaction,
            owner,
            block,
            text,
            search_chunk_ordinal(block.ordinal, TEXT_CHUNK_OFFSET)?,
            correction_by_kind.get("text"),
            timestamp,
        )
        .await?;
    }
    if let Some(latex) = block.latex.as_deref() {
        insert_correctable_search_value(
            transaction,
            owner,
            block,
            latex,
            search_chunk_ordinal(block.ordinal, LATEX_CHUNK_OFFSET)?,
            correction_by_kind.get("latex"),
            timestamp,
        )
        .await?;
    }
    if let Some(cells) = &block.table_cells {
        let table_text = cells
            .iter()
            .map(|cell| cell.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        insert_search_chunk(
            transaction,
            SearchChunkInsert {
                owner,
                source: ContentSource::AiTranscribed,
                correction_id: None,
                ordinal: search_chunk_ordinal(block.ordinal, TABLE_CHUNK_OFFSET)?,
                text: &table_text,
                locator_json: &locator_json(owner.page_number, block.bounds.as_ref())?,
                timestamp,
            },
        )
        .await?;
    }
    if let Some(description) = block.visual_description.as_deref() {
        insert_search_chunk(
            transaction,
            SearchChunkInsert {
                owner,
                source: ContentSource::AiDescription,
                correction_id: None,
                ordinal: search_chunk_ordinal(block.ordinal, DESCRIPTION_CHUNK_OFFSET)?,
                text: description,
                locator_json: &locator_json(owner.page_number, block.bounds.as_ref())?,
                timestamp,
            },
        )
        .await?;
    }
    Ok(())
}

struct CorrectionSearchState {
    id: Uuid,
    corrected_value: String,
    conflict_state: IndexCorrectionConflictState,
}

async fn insert_correctable_search_value(
    connection: &mut SqliteConnection,
    owner: SearchOwner,
    block: &ValidatedBlock,
    provider_value: &str,
    ordinal: u32,
    correction: Option<&CorrectionSearchState>,
    timestamp: &str,
) -> AppResult<()> {
    let (source, correction_id, text) = match correction {
        Some(correction) if correction.conflict_state == IndexCorrectionConflictState::Conflict => {
            return Ok(());
        }
        Some(correction) => (
            ContentSource::UserCorrected,
            Some(correction.id),
            correction.corrected_value.as_str(),
        ),
        None => (ContentSource::AiTranscribed, None, provider_value),
    };
    let locator = locator_json(owner.page_number, block.bounds.as_ref())?;
    insert_search_chunk(
        connection,
        SearchChunkInsert {
            owner,
            source,
            correction_id,
            ordinal,
            text,
            locator_json: &locator,
            timestamp,
        },
    )
    .await
}

pub(crate) struct SearchChunkInsert<'a> {
    owner: SearchOwner,
    pub source: ContentSource,
    pub correction_id: Option<Uuid>,
    pub ordinal: u32,
    pub text: &'a str,
    pub locator_json: &'a str,
    pub timestamp: &'a str,
}

pub(crate) async fn insert_search_chunk(
    connection: &mut SqliteConnection,
    input: SearchChunkInsert<'_>,
) -> AppResult<()> {
    if input.text.trim().is_empty() {
        return Err(AppError::new(AppErrorCode::InvalidInput));
    }
    let chunk_id = stable_index_search_chunk_id(input.owner.block_id, input.source, input.ordinal);
    let token_estimate = u32::try_from(input.text.chars().count().div_ceil(4).max(1))
        .map_err(|_| database_contract_error())?;
    sqlx::query(
        "INSERT INTO index_search_chunks (id, book_id, page_id, block_id, correction_id, ordinal, source, text, locator_json, token_estimate, content_version, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(chunk_id.to_string())
    .bind(input.owner.book_id.to_string())
    .bind(input.owner.page_id.to_string())
    .bind(input.owner.block_id.to_string())
    .bind(input.correction_id.map(|id| id.to_string()))
    .bind(i64::from(input.ordinal))
    .bind(input.source.as_str())
    .bind(input.text)
    .bind(input.locator_json)
    .bind(i64::from(token_estimate))
    .bind(i64::from(input.owner.content_version))
    .bind(input.timestamp)
    .execute(connection)
    .await?;
    Ok(())
}

pub(crate) fn correction_target_value(
    block: &ValidatedBlock,
    kind: IndexCorrectionValueKind,
) -> Option<&str> {
    match kind {
        IndexCorrectionValueKind::Text => block.plain_text.as_deref(),
        IndexCorrectionValueKind::Latex => block.latex.as_deref(),
    }
}

pub(crate) fn sha256_text(value: &str) -> String {
    hex_sha256(value.as_bytes())
}

pub(crate) fn search_chunk_ordinal(block_ordinal: u32, offset: u32) -> AppResult<u32> {
    block_ordinal
        .checked_mul(CHUNKS_PER_BLOCK)
        .and_then(|base| base.checked_add(offset))
        .filter(|ordinal| *ordinal <= 100_000)
        .ok_or_else(|| AppError::new(AppErrorCode::InvalidInput))
}

pub(crate) fn locator_json(page_number: u32, bounds: Option<&NormalizedRect>) -> AppResult<String> {
    let rects_by_page = bounds.map(|bounds| BTreeMap::from([(page_number, vec![bounds.clone()])]));
    let locator = DocumentLocator::pdf(page_number, page_number, rects_by_page)
        .map_err(|_| AppError::new(AppErrorCode::InvalidInput))?;
    serde_json::to_string(&locator).map_err(AppError::database)
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

fn sha256_serialized(value: &impl Serialize) -> AppResult<String> {
    let bytes = serde_json::to_vec(value).map_err(AppError::database)?;
    Ok(hex_sha256(&bytes))
}

fn hex_sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn require_not_cancelled(cancellation: &CancellationToken) -> AppResult<()> {
    if cancellation.is_cancelled() {
        Err(AppError::new(AppErrorCode::ImportCancelled))
    } else {
        Ok(())
    }
}

fn parse_uuid(value: String) -> AppResult<Uuid> {
    Uuid::parse_str(&value).map_err(|_| database_contract_error())
}

fn parse_value_kind(value: &str) -> AppResult<IndexCorrectionValueKind> {
    IndexCorrectionValueKind::from_database(value).ok_or_else(database_contract_error)
}

fn parse_conflict_state(value: &str) -> AppResult<IndexCorrectionConflictState> {
    IndexCorrectionConflictState::from_database(value).ok_or_else(database_contract_error)
}

const fn conflict_state_database(value: IndexCorrectionConflictState) -> &'static str {
    match value {
        IndexCorrectionConflictState::Active => "active",
        IndexCorrectionConflictState::Conflict => "conflict",
    }
}

const fn correction_kind_database(value: IndexCorrectionValueKind) -> &'static str {
    match value {
        IndexCorrectionValueKind::Text => "text",
        IndexCorrectionValueKind::Latex => "latex",
    }
}

fn require_one_row(rows_affected: u64) -> AppResult<()> {
    if rows_affected == 1 {
        Ok(())
    } else {
        Err(AppError::new(AppErrorCode::RequestConflict))
    }
}

fn database_contract_error() -> AppError {
    AppError::new(AppErrorCode::DatabaseError)
}
