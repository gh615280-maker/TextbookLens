use std::fmt;

use chrono::{DateTime, SecondsFormat, Utc};
use sqlx::{Row, SqlitePool, sqlite::SqliteRow};
use uuid::Uuid;

use crate::{
    domain::{
        IndexFailureCode, IndexPageBlockKind, IndexPageBlockReviewDto, IndexPageReviewDto,
        IndexPageStatus, IndexPageStatusCountsDto, IndexQualityReason, IndexReviewReason,
        IndexRunAggregateDto, IndexRunStatus, IndexTableCellDto, NormalizedRect, SafeIndexErrorDto,
        stable_index_page_id,
    },
    errors::{AppError, AppErrorCode, AppResult},
};

pub struct CreateIndexRun {
    pub book_id: Uuid,
    pub provider_profile_id: Uuid,
    pub analysis_schema_version: String,
    pub render_version: String,
    pub parser_version: String,
}

pub struct CreateIndexPage {
    pub run_id: Uuid,
    pub page_number: u32,
    pub quality_reason: IndexQualityReason,
    pub local_text_sha256: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IndexPageOwnership {
    pub page_id: Uuid,
    pub run_id: Uuid,
    pub book_id: Uuid,
    pub attempt_id: Option<Uuid>,
    pub status: IndexPageStatus,
}

pub struct EncryptedRemoteResourceReference(String);

impl EncryptedRemoteResourceReference {
    pub fn new(value: String) -> AppResult<Self> {
        if !(8..=4096).contains(&value.len())
            || !value.starts_with("enc:v1:")
            || value
                .chars()
                .any(|character| character == '\0' || character == '\r' || character == '\n')
        {
            return Err(AppError::new(AppErrorCode::InvalidInput));
        }
        Ok(Self(value))
    }

    fn database_value(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for EncryptedRemoteResourceReference {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("EncryptedRemoteResourceReference([REDACTED])")
    }
}

pub async fn create_index_run(pool: &SqlitePool, input: CreateIndexRun) -> AppResult<Uuid> {
    validate_version(&input.analysis_schema_version)?;
    validate_version(&input.render_version)?;
    validate_version(&input.parser_version)?;

    let book = sqlx::query("SELECT sha256, format, import_status FROM books WHERE id = ?")
        .bind(input.book_id.to_string())
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| AppError::new(AppErrorCode::NotFound))?;
    if book.try_get::<String, _>("format")? != "pdf"
        || book.try_get::<String, _>("import_status")? != "ready"
    {
        return Err(AppError::new(AppErrorCode::BookNotReady));
    }
    let source_sha256 = book
        .try_get::<Option<String>, _>("sha256")?
        .ok_or_else(|| AppError::new(AppErrorCode::BookNotReady))?;

    let profile = sqlx::query("SELECT provider_kind, model_id FROM provider_profiles WHERE id = ?")
        .bind(input.provider_profile_id.to_string())
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| AppError::new(AppErrorCode::NotFound))?;
    let provider_kind: String = profile.try_get("provider_kind")?;
    let model_id: String = profile.try_get("model_id")?;

    let run_id = Uuid::new_v4();
    let timestamp = database_timestamp(Utc::now());
    sqlx::query(
        "INSERT INTO index_runs (id, book_id, source_sha256, provider_profile_id, provider_kind, model_id, analysis_schema_version, render_version, parser_version, status, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 'running', ?, ?)",
    )
    .bind(run_id.to_string())
    .bind(input.book_id.to_string())
    .bind(source_sha256)
    .bind(input.provider_profile_id.to_string())
    .bind(provider_kind)
    .bind(model_id)
    .bind(input.analysis_schema_version)
    .bind(input.render_version)
    .bind(input.parser_version)
    .bind(&timestamp)
    .bind(&timestamp)
    .execute(pool)
    .await?;
    Ok(run_id)
}

pub async fn create_index_page(
    pool: &SqlitePool,
    input: CreateIndexPage,
) -> AppResult<IndexPageOwnership> {
    if input.page_number == 0 || input.page_number > 1_000_000 {
        return Err(AppError::new(AppErrorCode::InvalidInput));
    }
    if let Some(hash) = &input.local_text_sha256 {
        validate_sha256(hash)?;
    }

    let run = sqlx::query("SELECT book_id, status FROM index_runs WHERE id = ?")
        .bind(input.run_id.to_string())
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| AppError::new(AppErrorCode::NotFound))?;
    if run.try_get::<String, _>("status")? != IndexRunStatus::Running.as_str() {
        return Err(AppError::new(AppErrorCode::RequestConflict));
    }
    let book_id = parse_uuid(run.try_get::<String, _>("book_id")?)?;
    let page_id = stable_index_page_id(input.run_id, input.page_number);
    let (status, attempt_id, attempt_count) =
        if input.quality_reason == IndexQualityReason::ReliableText {
            (IndexPageStatus::NotRequired, None, 0_i64)
        } else {
            (IndexPageStatus::Queued, Some(Uuid::new_v4()), 1_i64)
        };
    let timestamp = database_timestamp(Utc::now());
    sqlx::query(
        "INSERT INTO index_pages (id, run_id, book_id, page_number, quality_reason, status, attempt_id, attempt_count, local_text_sha256, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(page_id.to_string())
    .bind(input.run_id.to_string())
    .bind(book_id.to_string())
    .bind(i64::from(input.page_number))
    .bind(input.quality_reason.as_str())
    .bind(status.as_str())
    .bind(attempt_id.map(|id| id.to_string()))
    .bind(attempt_count)
    .bind(input.local_text_sha256)
    .bind(&timestamp)
    .bind(&timestamp)
    .execute(pool)
    .await?;

    Ok(IndexPageOwnership {
        page_id,
        run_id: input.run_id,
        book_id,
        attempt_id,
        status,
    })
}

pub async fn get_run_aggregate(pool: &SqlitePool, run_id: Uuid) -> AppResult<IndexRunAggregateDto> {
    let run = sqlx::query("SELECT book_id, status, updated_at FROM index_runs WHERE id = ?")
        .bind(run_id.to_string())
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| AppError::new(AppErrorCode::NotFound))?;
    let book_id = parse_uuid(run.try_get::<String, _>("book_id")?)?;
    let control_status = parse_run_status(&run.try_get::<String, _>("status")?)?;
    let mut updated_at = parse_timestamp(&run.try_get::<String, _>("updated_at")?)?;

    let rows = sqlx::query("SELECT status, updated_at FROM index_pages WHERE run_id = ?")
        .bind(run_id.to_string())
        .fetch_all(pool)
        .await?;
    let mut pages = IndexPageStatusCountsDto::default();
    for row in rows {
        let status = parse_page_status(&row.try_get::<String, _>("status")?)?;
        pages.record(status);
        let page_updated_at = parse_timestamp(&row.try_get::<String, _>("updated_at")?)?;
        updated_at = updated_at.max(page_updated_at);
    }

    Ok(IndexRunAggregateDto {
        run_id,
        book_id,
        control_status,
        aggregate_status: pages.aggregate_status(),
        pages,
        updated_at,
    })
}

/// Resolves the latest persisted run for exactly one canonical book identifier.
/// The query deliberately returns no run when the book has never had indexing.
pub async fn find_current_run_aggregate_for_book(
    pool: &SqlitePool,
    book_id: Uuid,
) -> AppResult<Option<IndexRunAggregateDto>> {
    let run_id = sqlx::query_scalar::<_, String>(
        "SELECT id FROM index_runs WHERE book_id = ? ORDER BY updated_at DESC, id DESC LIMIT 1",
    )
    .bind(book_id.to_string())
    .fetch_optional(pool)
    .await?;

    let Some(run_id) = run_id else {
        return Ok(None);
    };
    let aggregate = get_run_aggregate(pool, parse_uuid(run_id)?).await?;
    if aggregate.book_id != book_id {
        return Err(AppError::new(AppErrorCode::DatabaseError));
    }
    Ok(Some(aggregate))
}

pub async fn list_page_reviews(
    pool: &SqlitePool,
    run_id: Uuid,
) -> AppResult<Vec<IndexPageReviewDto>> {
    let page_ids: Vec<String> = sqlx::query_scalar(
        "SELECT id FROM index_pages WHERE run_id = ? AND status IN ('needs_review', 'failed') ORDER BY page_number",
    )
    .bind(run_id.to_string())
    .fetch_all(pool)
    .await?;

    let mut reviews = Vec::with_capacity(page_ids.len());
    for page_id in page_ids {
        reviews.push(get_page_review(pool, parse_uuid(page_id)?).await?);
    }
    Ok(reviews)
}

pub async fn get_page_review(pool: &SqlitePool, page_id: Uuid) -> AppResult<IndexPageReviewDto> {
    let page = sqlx::query(
        "SELECT id, run_id, book_id, page_number, quality_reason, status, review_reason_code, safe_error_code, safe_error_message, retryable, content_version, updated_at FROM index_pages WHERE id = ?",
    )
    .bind(page_id.to_string())
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::new(AppErrorCode::NotFound))?;
    let content_version = to_u32(page.try_get::<i64, _>("content_version")?)?;

    let block_rows = sqlx::query(
        "SELECT id, ordinal, kind, plain_text, latex, table_json, visual_description, bounds_x, bounds_y, bounds_width, bounds_height, source, content_version FROM index_page_blocks WHERE page_id = ? AND content_version = ? ORDER BY ordinal",
    )
    .bind(page_id.to_string())
    .bind(i64::from(content_version))
    .fetch_all(pool)
    .await?;
    let blocks = block_rows
        .iter()
        .map(block_review_from_row)
        .collect::<AppResult<Vec<_>>>()?;
    let corrections = super::corrections::list_page_corrections(pool, page_id).await?;

    let safe_error = match (
        page.try_get::<Option<String>, _>("safe_error_code")?,
        page.try_get::<Option<String>, _>("safe_error_message")?,
    ) {
        (None, None) => None,
        (Some(code), Some(message)) => Some(SafeIndexErrorDto {
            code: IndexFailureCode::from_database(&code).ok_or_else(database_contract_error)?,
            message,
            retryable: page.try_get::<i64, _>("retryable")? == 1,
        }),
        _ => return Err(database_contract_error()),
    };
    let review_reason = page
        .try_get::<Option<String>, _>("review_reason_code")?
        .map(|reason| IndexReviewReason::from_database(&reason).ok_or_else(database_contract_error))
        .transpose()?;

    Ok(IndexPageReviewDto {
        id: parse_uuid(page.try_get::<String, _>("id")?)?,
        run_id: parse_uuid(page.try_get::<String, _>("run_id")?)?,
        book_id: parse_uuid(page.try_get::<String, _>("book_id")?)?,
        page_number: to_u32(page.try_get::<i64, _>("page_number")?)?,
        quality_reason: IndexQualityReason::from_database(
            &page.try_get::<String, _>("quality_reason")?,
        )
        .ok_or_else(database_contract_error)?,
        status: parse_page_status(&page.try_get::<String, _>("status")?)?,
        review_reason,
        safe_error,
        content_version,
        blocks,
        corrections,
        updated_at: parse_timestamp(&page.try_get::<String, _>("updated_at")?)?,
    })
}

pub async fn track_remote_resource(
    pool: &SqlitePool,
    page_id: Uuid,
    reference: EncryptedRemoteResourceReference,
) -> AppResult<Uuid> {
    let owner = sqlx::query(
        "SELECT p.run_id, p.book_id, r.provider_profile_id, r.provider_kind FROM index_pages p JOIN index_runs r ON r.id = p.run_id AND r.book_id = p.book_id WHERE p.id = ?",
    )
    .bind(page_id.to_string())
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::new(AppErrorCode::NotFound))?;
    let profile_id: Option<String> = owner.try_get("provider_profile_id")?;
    let resource_id = Uuid::new_v4();
    let timestamp = database_timestamp(Utc::now());
    sqlx::query(
        "INSERT INTO provider_remote_resources (id, book_id, run_id, page_id, provider_profile_id, provider_kind, encrypted_reference, cleanup_status, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, 'pending', ?, ?)",
    )
    .bind(resource_id.to_string())
    .bind(owner.try_get::<String, _>("book_id")?)
    .bind(owner.try_get::<String, _>("run_id")?)
    .bind(page_id.to_string())
    .bind(profile_id)
    .bind(owner.try_get::<String, _>("provider_kind")?)
    .bind(reference.database_value())
    .bind(&timestamp)
    .bind(&timestamp)
    .execute(pool)
    .await?;
    Ok(resource_id)
}

fn block_review_from_row(row: &SqliteRow) -> AppResult<IndexPageBlockReviewDto> {
    let bounds = match (
        row.try_get::<Option<f64>, _>("bounds_x")?,
        row.try_get::<Option<f64>, _>("bounds_y")?,
        row.try_get::<Option<f64>, _>("bounds_width")?,
        row.try_get::<Option<f64>, _>("bounds_height")?,
    ) {
        (None, None, None, None) => None,
        (Some(x), Some(y), Some(width), Some(height)) => {
            Some(NormalizedRect::new(x, y, width, height).map_err(|_| database_contract_error())?)
        }
        _ => return Err(database_contract_error()),
    };
    let table_cells = row
        .try_get::<Option<String>, _>("table_json")?
        .map(|value| serde_json::from_str::<Vec<IndexTableCellDto>>(&value))
        .transpose()
        .map_err(AppError::database)?;
    let source = crate::domain::ContentSource::from_database(&row.try_get::<String, _>("source")?)
        .ok_or_else(database_contract_error)?;
    if source == crate::domain::ContentSource::LocalText
        || source == crate::domain::ContentSource::UserCorrected
    {
        return Err(database_contract_error());
    }

    Ok(IndexPageBlockReviewDto {
        id: parse_uuid(row.try_get::<String, _>("id")?)?,
        ordinal: to_u32(row.try_get::<i64, _>("ordinal")?)?,
        kind: IndexPageBlockKind::from_database(&row.try_get::<String, _>("kind")?)
            .ok_or_else(database_contract_error)?,
        source,
        plain_text: row.try_get("plain_text")?,
        latex: row.try_get("latex")?,
        table_cells,
        visual_description: row.try_get("visual_description")?,
        bounds,
        content_version: to_u32(row.try_get::<i64, _>("content_version")?)?,
    })
}

fn validate_version(value: &str) -> AppResult<()> {
    if !(1..=64).contains(&value.trim().len()) || value.contains(['\r', '\n']) {
        Err(AppError::new(AppErrorCode::InvalidInput))
    } else {
        Ok(())
    }
}

fn validate_sha256(value: &str) -> AppResult<()> {
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

pub(crate) fn database_timestamp(timestamp: DateTime<Utc>) -> String {
    timestamp.to_rfc3339_opts(SecondsFormat::Millis, true)
}

pub(crate) fn parse_timestamp(value: &str) -> AppResult<DateTime<Utc>> {
    let timestamp = DateTime::parse_from_rfc3339(value)
        .map_err(|_| database_contract_error())?
        .with_timezone(&Utc);
    if database_timestamp(timestamp) != value {
        return Err(database_contract_error());
    }
    Ok(timestamp)
}

pub(crate) fn parse_uuid(value: String) -> AppResult<Uuid> {
    Uuid::parse_str(&value).map_err(|_| database_contract_error())
}

pub(crate) fn parse_page_status(value: &str) -> AppResult<IndexPageStatus> {
    IndexPageStatus::from_database(value).ok_or_else(database_contract_error)
}

pub(crate) fn parse_run_status(value: &str) -> AppResult<IndexRunStatus> {
    IndexRunStatus::from_database(value).ok_or_else(database_contract_error)
}

pub(crate) fn to_u32(value: i64) -> AppResult<u32> {
    u32::try_from(value).map_err(|_| database_contract_error())
}

pub(crate) fn database_contract_error() -> AppError {
    AppError::new(AppErrorCode::DatabaseError)
}
