use chrono::{DateTime, Utc};
use sqlx::{Row, SqlitePool, sqlite::SqliteRow};
use uuid::Uuid;

use crate::{
    domain::{
        BookFormat, BookIndexAggregateStatus, BookSummary, DocumentLocator, ImportErrorStage,
        ImportStatus, IndexAggregate,
    },
    errors::{AppError, AppErrorCode, AppResult},
};

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReaderSection {
    pub id: Uuid,
    pub parent_id: Option<Uuid>,
    pub ordinal: u32,
    pub title: String,
    pub locator: DocumentLocator,
}

pub async fn list_reader_sections(
    pool: &SqlitePool,
    book_id: Uuid,
) -> AppResult<Vec<ReaderSection>> {
    let status: Option<String> = sqlx::query_scalar("SELECT import_status FROM books WHERE id = ?")
        .bind(book_id.to_string())
        .fetch_optional(pool)
        .await?;
    match status.as_deref() {
        Some("ready") => {}
        Some(_) => return Err(AppError::new(AppErrorCode::BookNotReady)),
        None => return Err(AppError::new(AppErrorCode::NotFound)),
    }
    sqlx::query("SELECT id, parent_id, ordinal, title, locator_json FROM sections WHERE book_id = ? ORDER BY ordinal")
        .bind(book_id.to_string()).fetch_all(pool).await?.into_iter().map(|row| Ok(ReaderSection {
            id: Uuid::parse_str(&row.try_get::<String, _>("id")?).map_err(|_| AppError::new(AppErrorCode::DatabaseError))?,
            parent_id: row.try_get::<Option<String>, _>("parent_id")?.map(|id| Uuid::parse_str(&id).map_err(|_| AppError::new(AppErrorCode::DatabaseError))).transpose()?,
            ordinal: row.try_get::<i64, _>("ordinal")? as u32,
            title: row.try_get("title")?,
            locator: serde_json::from_str(&row.try_get::<String, _>("locator_json")?).map_err(AppError::database)?,
        })).collect()
}

#[derive(Clone, Debug)]
pub struct BookRecord {
    pub summary: BookSummary,
    pub sha256: Option<String>,
    pub stored_path: Option<String>,
}

#[derive(Clone)]
pub struct BookDeletePlan {
    pub book_id: Uuid,
    pub format: BookFormat,
    pub import_status: ImportStatus,
    pub stored_path: Option<String>,
    pub page_ids: Vec<Uuid>,
    pub has_nonterminal_indexing: bool,
}

impl std::fmt::Debug for BookDeletePlan {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("BookDeletePlan")
            .field("book_id", &"<redacted>")
            .field("format", &self.format)
            .field("import_status", &self.import_status)
            .field(
                "stored_path",
                &self.stored_path.as_ref().map(|_| "<redacted>"),
            )
            .field("page_count", &self.page_ids.len())
            .field("has_nonterminal_indexing", &self.has_nonterminal_indexing)
            .finish()
    }
}

pub async fn load_delete_plan(pool: &SqlitePool, book_id: Uuid) -> AppResult<BookDeletePlan> {
    let row =
        sqlx::query("SELECT format, import_status, sha256, stored_path FROM books WHERE id = ?")
            .bind(book_id.to_string())
            .fetch_optional(pool)
            .await?
            .ok_or_else(|| AppError::new(AppErrorCode::NotFound))?;
    let sha256: Option<String> = row.try_get("sha256")?;
    let stored_path: Option<String> = row.try_get("stored_path")?;
    if sha256.is_some() != stored_path.is_some() {
        return Err(AppError::new(AppErrorCode::DatabaseError));
    }

    let maximum_page_ids = crate::maintenance::journal::MAX_DELETE_JOURNAL_ENTRIES;
    let page_query_limit = i64::try_from(
        maximum_page_ids
            .checked_add(1)
            .ok_or_else(|| AppError::new(AppErrorCode::DatabaseError))?,
    )
    .map_err(|_| AppError::new(AppErrorCode::DatabaseError))?;
    let page_ids = sqlx::query_scalar::<_, String>(
        "SELECT id FROM index_pages WHERE book_id = ? ORDER BY id LIMIT ?",
    )
    .bind(book_id.to_string())
    .bind(page_query_limit)
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|id| Uuid::parse_str(&id).map_err(|_| AppError::new(AppErrorCode::DatabaseError)))
    .collect::<AppResult<Vec<_>>>()?;
    if page_ids.len() > maximum_page_ids {
        return Err(AppError::new(AppErrorCode::RequestConflict));
    }
    if page_ids.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err(AppError::new(AppErrorCode::DatabaseError));
    }

    let has_nonterminal_indexing: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM index_runs WHERE book_id = ? AND status IN ('running', 'paused', 'cancelling')) OR EXISTS(SELECT 1 FROM index_pages WHERE book_id = ? AND status IN ('queued', 'rendering', 'sending', 'parsing', 'validating'))",
    )
    .bind(book_id.to_string())
    .bind(book_id.to_string())
    .fetch_one(pool)
    .await?;

    Ok(BookDeletePlan {
        book_id,
        format: parse_format(&row.try_get::<String, _>("format")?)?,
        import_status: parse_status(&row.try_get::<String, _>("import_status")?)?,
        stored_path,
        page_ids,
        has_nonterminal_indexing,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CompleteDeleteStep {
    BeforeBegin,
    AfterBegin,
    AfterRemoteDetach,
    AfterBookDelete,
    AfterVerification,
    BeforeCommit,
    AfterCommit,
}

impl CompleteDeleteStep {
    pub const ALL: [Self; 7] = [
        Self::BeforeBegin,
        Self::AfterBegin,
        Self::AfterRemoteDetach,
        Self::AfterBookDelete,
        Self::AfterVerification,
        Self::BeforeCommit,
        Self::AfterCommit,
    ];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompleteDeleteInjectedCrash {
    step: CompleteDeleteStep,
}

impl CompleteDeleteInjectedCrash {
    pub fn at(step: CompleteDeleteStep) -> Self {
        Self { step }
    }

    pub fn step(self) -> CompleteDeleteStep {
        self.step
    }
}

pub trait CompleteDeleteFaultInjector: Send + Sync {
    fn checkpoint(&self, step: CompleteDeleteStep) -> Result<(), CompleteDeleteInjectedCrash>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct NoCompleteDeleteFault;

impl CompleteDeleteFaultInjector for NoCompleteDeleteFault {
    fn checkpoint(&self, _step: CompleteDeleteStep) -> Result<(), CompleteDeleteInjectedCrash> {
        Ok(())
    }
}

pub enum CompleteDeleteError {
    App(AppError),
    Injected(CompleteDeleteInjectedCrash),
}

impl std::fmt::Debug for CompleteDeleteError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::App(error) => formatter
                .debug_struct("CompleteDeleteError")
                .field("code", &error.code)
                .finish(),
            Self::Injected(crash) => formatter
                .debug_struct("CompleteDeleteInjectedCrash")
                .field("step", &crash.step)
                .finish(),
        }
    }
}

impl From<AppError> for CompleteDeleteError {
    fn from(error: AppError) -> Self {
        Self::App(error)
    }
}

impl From<sqlx::Error> for CompleteDeleteError {
    fn from(error: sqlx::Error) -> Self {
        Self::App(AppError::from(error))
    }
}

#[derive(Clone)]
pub struct CompleteDeleteResult {
    detached_remote_resource_ids: Vec<Uuid>,
}

impl CompleteDeleteResult {
    pub fn detached_remote_resource_ids(&self) -> &[Uuid] {
        &self.detached_remote_resource_ids
    }
}

impl std::fmt::Debug for CompleteDeleteResult {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CompleteDeleteResult")
            .field(
                "detached_remote_resource_count",
                &self.detached_remote_resource_ids.len(),
            )
            .finish()
    }
}

pub async fn complete_delete(
    pool: &SqlitePool,
    book_id: Uuid,
    fault: &dyn CompleteDeleteFaultInjector,
) -> Result<CompleteDeleteResult, CompleteDeleteError> {
    checkpoint_complete_delete(fault, CompleteDeleteStep::BeforeBegin)?;
    let mut transaction = pool.begin().await?;
    checkpoint_complete_delete(fault, CompleteDeleteStep::AfterBegin)?;

    let lock = sqlx::query(
        "UPDATE books SET updated_at = updated_at WHERE id = ? AND import_status IN ('ready', 'failed')",
    )
    .bind(book_id.to_string())
    .execute(&mut *transaction)
    .await?;
    if lock.rows_affected() != 1 {
        let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM books WHERE id = ?)")
            .bind(book_id.to_string())
            .fetch_one(&mut *transaction)
            .await?;
        return Err(CompleteDeleteError::App(AppError::new(if exists {
            AppErrorCode::RequestConflict
        } else {
            AppErrorCode::NotFound
        })));
    }

    let maximum_remote_ids = crate::maintenance::journal::MAX_DELETE_JOURNAL_ENTRIES;
    let remote_query_limit = i64::try_from(
        maximum_remote_ids
            .checked_add(1)
            .ok_or_else(|| AppError::new(AppErrorCode::DatabaseError))?,
    )
    .map_err(|_| AppError::new(AppErrorCode::DatabaseError))?;
    let detached_remote_resource_ids = sqlx::query_scalar::<_, String>(
        "SELECT id FROM provider_remote_resources WHERE book_id = ? AND cleanup_status IN ('pending', 'cleaning', 'failed') ORDER BY id LIMIT ?",
    )
    .bind(book_id.to_string())
    .bind(remote_query_limit)
    .fetch_all(&mut *transaction)
    .await?
    .into_iter()
    .map(|id| Uuid::parse_str(&id).map_err(|_| AppError::new(AppErrorCode::DatabaseError)))
    .collect::<AppResult<Vec<_>>>()?;
    if detached_remote_resource_ids.len() > maximum_remote_ids {
        return Err(CompleteDeleteError::App(AppError::new(
            AppErrorCode::RequestConflict,
        )));
    }
    let detached = sqlx::query(
        "UPDATE provider_remote_resources SET book_id = NULL, run_id = NULL, page_id = NULL WHERE book_id = ? AND cleanup_status IN ('pending', 'cleaning', 'failed')",
    )
    .bind(book_id.to_string())
    .execute(&mut *transaction)
    .await?;
    if detached.rows_affected()
        != u64::try_from(detached_remote_resource_ids.len())
            .map_err(|_| AppError::new(AppErrorCode::DatabaseError))?
    {
        return Err(CompleteDeleteError::App(AppError::new(
            AppErrorCode::DatabaseError,
        )));
    }
    checkpoint_complete_delete(fault, CompleteDeleteStep::AfterRemoteDetach)?;

    let deleted =
        sqlx::query("DELETE FROM books WHERE id = ? AND import_status IN ('ready', 'failed')")
            .bind(book_id.to_string())
            .execute(&mut *transaction)
            .await?;
    if deleted.rows_affected() != 1 {
        return Err(CompleteDeleteError::App(AppError::new(
            AppErrorCode::RequestConflict,
        )));
    }
    checkpoint_complete_delete(fault, CompleteDeleteStep::AfterBookDelete)?;

    verify_complete_delete(&mut transaction, book_id).await?;
    checkpoint_complete_delete(fault, CompleteDeleteStep::AfterVerification)?;
    checkpoint_complete_delete(fault, CompleteDeleteStep::BeforeCommit)?;
    transaction.commit().await?;
    checkpoint_complete_delete(fault, CompleteDeleteStep::AfterCommit)?;
    Ok(CompleteDeleteResult {
        detached_remote_resource_ids,
    })
}

fn checkpoint_complete_delete(
    fault: &dyn CompleteDeleteFaultInjector,
    step: CompleteDeleteStep,
) -> Result<(), CompleteDeleteError> {
    fault
        .checkpoint(step)
        .map_err(CompleteDeleteError::Injected)
}

async fn verify_complete_delete(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    book_id: Uuid,
) -> Result<(), CompleteDeleteError> {
    const RELATED_COUNTS: &[&str] = &[
        "SELECT COUNT(*) FROM sections WHERE book_id = ?",
        "SELECT COUNT(*) FROM blocks WHERE book_id = ?",
        "SELECT COUNT(*) FROM search_chunks WHERE book_id = ?",
        "SELECT COUNT(*) FROM conversations WHERE book_id = ?",
        "SELECT COUNT(*) FROM annotations WHERE book_id = ?",
        "SELECT COUNT(*) FROM index_runs WHERE book_id = ?",
        "SELECT COUNT(*) FROM index_pages WHERE book_id = ?",
        "SELECT COUNT(*) FROM index_page_blocks WHERE book_id = ?",
        "SELECT COUNT(*) FROM index_corrections WHERE book_id = ?",
        "SELECT COUNT(*) FROM index_search_chunks WHERE book_id = ?",
        "SELECT COUNT(*) FROM provider_remote_resources WHERE book_id = ?",
    ];
    for query in RELATED_COUNTS {
        let count: i64 = sqlx::query_scalar(*query)
            .bind(book_id.to_string())
            .fetch_one(&mut **transaction)
            .await?;
        if count != 0 {
            return Err(CompleteDeleteError::App(AppError::new(
                AppErrorCode::DatabaseError,
            )));
        }
    }
    let orphan_messages: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM messages WHERE NOT EXISTS (SELECT 1 FROM conversations WHERE conversations.id = messages.conversation_id)",
    )
    .fetch_one(&mut **transaction)
    .await?;
    let foreign_key_violations = sqlx::query("PRAGMA foreign_key_check")
        .fetch_all(&mut **transaction)
        .await?;
    if orphan_messages != 0 || !foreign_key_violations.is_empty() {
        return Err(CompleteDeleteError::App(AppError::new(
            AppErrorCode::DatabaseError,
        )));
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub enum HashClaim {
    Claimed(BookSummary),
    Duplicate(BookSummary),
}

pub async fn insert_queued(
    pool: &SqlitePool,
    id: Uuid,
    format: &BookFormat,
    original_filename: &str,
) -> AppResult<()> {
    let title = std::path::Path::new(original_filename)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| !stem.is_empty())
        .unwrap_or(original_filename);
    sqlx::query(
        "INSERT INTO books (id, title, format, original_filename, import_status, created_at, updated_at) VALUES (?, ?, ?, ?, 'queued', strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
    )
    .bind(id.to_string())
    .bind(title)
    .bind(format_name(format))
    .bind(original_filename)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn start_copying(pool: &SqlitePool, id: Uuid) -> AppResult<()> {
    let result = sqlx::query(
        "UPDATE books SET import_status = 'copying', updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ? AND import_status = 'queued'",
    )
    .bind(id.to_string())
    .execute(pool)
    .await?;
    ensure_changed(result.rows_affected())
}

pub async fn claim_hash(
    pool: &SqlitePool,
    id: Uuid,
    sha256: &str,
    stored_path: &str,
    original_filename: Option<&str>,
) -> AppResult<HashClaim> {
    let mut transaction = pool.begin().await?;
    let updated = if let Some(original_filename) = original_filename {
        sqlx::query(
            "UPDATE books SET sha256 = ?, stored_path = ?, original_filename = ?, import_status = 'parsing', import_error_code = NULL, import_error_message = NULL, import_error_stage = NULL, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ? AND import_status = 'copying'",
        )
        .bind(sha256)
        .bind(stored_path)
        .bind(original_filename)
        .bind(id.to_string())
        .execute(&mut *transaction)
        .await
    } else {
        sqlx::query(
            "UPDATE books SET sha256 = ?, stored_path = ?, import_status = 'parsing', import_error_code = NULL, import_error_message = NULL, import_error_stage = NULL, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ? AND import_status = 'copying'",
        )
        .bind(sha256)
        .bind(stored_path)
        .bind(id.to_string())
        .execute(&mut *transaction)
        .await
    };

    let updated = match updated {
        Ok(updated) => updated,
        Err(error) if is_sha256_unique_violation(&error) => {
            let duplicate = sqlx::query(BOOK_SUMMARY_PROJECTION)
                .bind(Option::<String>::None)
                .bind(Option::<String>::None)
                .bind(Some(sha256))
                .bind(sha256)
                .bind(id.to_string())
                .fetch_optional(&mut *transaction)
                .await?
                .map(|row| row_to_record(&row))
                .transpose()?;
            if let Some(duplicate) = duplicate {
                sqlx::query("DELETE FROM books WHERE id = ?")
                    .bind(id.to_string())
                    .execute(&mut *transaction)
                    .await?;
                transaction.commit().await?;
                return Ok(HashClaim::Duplicate(duplicate.summary));
            }
            return Err(AppError::database(error));
        }
        Err(error) => return Err(AppError::database(error)),
    };

    if updated.rows_affected() == 0 {
        return Err(AppError::new(AppErrorCode::RequestConflict));
    }

    let book = fetch_in_transaction(&mut transaction, id).await?;
    transaction.commit().await?;
    Ok(HashClaim::Claimed(book.summary))
}

pub async fn get(pool: &SqlitePool, id: Uuid) -> AppResult<BookRecord> {
    let row = sqlx::query(BOOK_SUMMARY_PROJECTION)
        .bind(Some(id.to_string()))
        .bind(id.to_string())
        .bind(Option::<String>::None)
        .bind(Option::<String>::None)
        .bind(Option::<String>::None)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| AppError::new(AppErrorCode::NotFound))?;
    row_to_record(&row)
}

pub async fn list(pool: &SqlitePool) -> AppResult<Vec<BookSummary>> {
    sqlx::query(BOOK_SUMMARY_PROJECTION)
        .bind(Option::<String>::None)
        .bind(Option::<String>::None)
        .bind(Option::<String>::None)
        .bind(Option::<String>::None)
        .bind(Option::<String>::None)
        .fetch_all(pool)
        .await?
        .iter()
        .map(row_to_record)
        .map(|record| record.map(|record| record.summary))
        .collect()
}

pub async fn mark_failed(
    pool: &SqlitePool,
    id: Uuid,
    stage: ImportErrorStage,
    code: AppErrorCode,
    message: &str,
    clear_owned_source: bool,
) -> AppResult<()> {
    let result = if clear_owned_source {
        sqlx::query(
            "UPDATE books SET import_status = 'failed', import_error_code = ?, import_error_message = ?, import_error_stage = ?, sha256 = NULL, stored_path = NULL, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?",
        )
        .bind(code_name(code))
        .bind(message)
        .bind(stage_name(&stage))
        .bind(id.to_string())
        .execute(pool)
        .await?
    } else {
        sqlx::query(
            "UPDATE books SET import_status = 'failed', import_error_code = ?, import_error_message = ?, import_error_stage = ?, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?",
        )
        .bind(code_name(code))
        .bind(message)
        .bind(stage_name(&stage))
        .bind(id.to_string())
        .execute(pool)
        .await?
    };
    if result.rows_affected() == 0 {
        return Err(AppError::new(AppErrorCode::NotFound));
    }
    Ok(())
}

pub async fn begin_parse(
    pool: &SqlitePool,
    id: Uuid,
    title: &str,
    author: Option<&str>,
    language: Option<&str>,
) -> AppResult<()> {
    let result = sqlx::query(
        "UPDATE books SET title = ?, author = ?, language = ?, import_status = 'parsing', import_error_code = NULL, import_error_message = NULL, import_error_stage = NULL, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ? AND import_status = 'parsing'",
    )
    .bind(title)
    .bind(author)
    .bind(language)
    .bind(id.to_string())
    .execute(pool)
    .await?;
    ensure_changed(result.rows_affected())
}

pub async fn require_parsing(pool: &SqlitePool, id: Uuid) -> AppResult<()> {
    let status: Option<String> = sqlx::query_scalar("SELECT import_status FROM books WHERE id = ?")
        .bind(id.to_string())
        .fetch_optional(pool)
        .await?;
    match status.as_deref() {
        Some("parsing") => Ok(()),
        Some(_) => Err(AppError::new(AppErrorCode::BookNotReady)),
        None => Err(AppError::new(AppErrorCode::NotFound)),
    }
}

pub async fn finalize(pool: &SqlitePool, id: Uuid) -> AppResult<BookSummary> {
    let result = sqlx::query(
        "UPDATE books SET import_status = 'ready', import_error_code = NULL, import_error_message = NULL, import_error_stage = NULL, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ? AND import_status = 'indexing'",
    )
    .bind(id.to_string())
    .execute(pool)
    .await?;
    ensure_changed(result.rows_affected())?;
    Ok(get(pool, id).await?.summary)
}

pub async fn begin_indexing(pool: &SqlitePool, id: Uuid) -> AppResult<()> {
    let result = sqlx::query(
        "UPDATE books SET import_status = 'indexing', updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ? AND import_status = 'parsing'",
    )
    .bind(id.to_string())
    .execute(pool)
    .await?;
    ensure_changed(result.rows_affected())
}

pub async fn reset_failed_for_retry(pool: &SqlitePool, id: Uuid) -> AppResult<BookSummary> {
    let result = sqlx::query(
        "UPDATE books SET import_status = 'parsing', import_error_code = NULL, import_error_message = NULL, import_error_stage = NULL, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ? AND import_status = 'failed' AND sha256 IS NOT NULL AND stored_path IS NOT NULL",
    )
    .bind(id.to_string())
    .execute(pool)
    .await?;
    ensure_changed(result.rows_affected())?;
    Ok(get(pool, id).await?.summary)
}

pub async fn prepare_failed_for_replacement(pool: &SqlitePool, id: Uuid) -> AppResult<()> {
    let result = sqlx::query(
        "UPDATE books SET sha256 = NULL, stored_path = NULL, import_status = 'copying', import_error_code = NULL, import_error_message = NULL, import_error_stage = NULL, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ? AND import_status = 'failed'",
    )
    .bind(id.to_string())
    .execute(pool)
    .await?;
    ensure_changed(result.rows_affected())
}

pub async fn delete_failed(pool: &SqlitePool, id: Uuid) -> AppResult<()> {
    let result = sqlx::query("DELETE FROM books WHERE id = ? AND import_status = 'failed'")
        .bind(id.to_string())
        .execute(pool)
        .await?;
    ensure_changed(result.rows_affected())
}

async fn fetch_in_transaction(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    id: Uuid,
) -> AppResult<BookRecord> {
    let row = sqlx::query(BOOK_SUMMARY_PROJECTION)
        .bind(Some(id.to_string()))
        .bind(id.to_string())
        .bind(Option::<String>::None)
        .bind(Option::<String>::None)
        .bind(Option::<String>::None)
        .fetch_one(&mut **transaction)
        .await?;
    row_to_record(&row)
}

fn row_to_record(row: &SqliteRow) -> AppResult<BookRecord> {
    let id = Uuid::parse_str(row.try_get::<String, _>("id")?.as_str())
        .map_err(|_| AppError::new(AppErrorCode::DatabaseError))?;
    let format = parse_format(&row.try_get::<String, _>("format")?)?;
    let import_status = parse_status(&row.try_get::<String, _>("import_status")?)?;
    let created_at = parse_datetime(row.try_get("created_at")?)?;
    let updated_at = parse_datetime(row.try_get("updated_at")?)?;
    let last_opened_at = row
        .try_get::<Option<String>, _>("last_opened_at")?
        .map(parse_datetime)
        .transpose()?;
    let index_aggregate = index_aggregate_from_row(row)?;
    Ok(BookRecord {
        sha256: row.try_get("sha256")?,
        stored_path: row.try_get("stored_path")?,
        summary: BookSummary {
            id,
            title: row.try_get("title")?,
            original_filename: row.try_get("original_filename")?,
            author: row.try_get("author")?,
            language: row.try_get("language")?,
            format,
            import_status,
            import_error_code: row.try_get("import_error_code")?,
            import_error_message: row.try_get("import_error_message")?,
            import_error_stage: row
                .try_get::<Option<String>, _>("import_error_stage")?
                .map(|stage| parse_error_stage(&stage))
                .transpose()?,
            reading_progress: row.try_get("reading_progress")?,
            index_aggregate,
            created_at,
            updated_at,
            last_opened_at,
        },
    })
}

const BOOK_SUMMARY_PROJECTION: &str = r#"
WITH ranked_runs AS (
    SELECT
        id,
        book_id,
        ROW_NUMBER() OVER (PARTITION BY book_id ORDER BY updated_at DESC, id DESC) AS row_number
    FROM index_runs
), index_aggregates AS (
    SELECT
        run.book_id,
        COUNT(page.id) AS index_total_pages,
        COALESCE(SUM(CASE WHEN page.status = 'indexed' THEN 1 ELSE 0 END), 0) AS index_indexed_pages,
        COALESCE(SUM(CASE WHEN page.status = 'needs_review' THEN 1 ELSE 0 END), 0) AS index_review_pages,
        COALESCE(SUM(CASE WHEN page.status IN ('failed', 'cancelled') THEN 1 ELSE 0 END), 0) AS index_failed_pages,
        COALESCE(SUM(CASE WHEN page.status = 'not_required' THEN 1 ELSE 0 END), 0) AS index_not_required_pages,
        COALESCE(SUM(CASE WHEN page.status IN ('queued', 'rendering', 'sending', 'parsing', 'validating') THEN 1 ELSE 0 END), 0) AS index_unresolved_pages,
        COALESCE(SUM(CASE WHEN page.status NOT IN ('not_required', 'queued', 'rendering', 'sending', 'parsing', 'validating', 'indexed', 'needs_review', 'failed', 'cancelled') THEN 1 ELSE 0 END), 0) AS index_invalid_statuses
    FROM ranked_runs run
    LEFT JOIN index_pages page ON page.run_id = run.id AND page.book_id = run.book_id
    WHERE run.row_number = 1
    GROUP BY run.book_id
)
SELECT b.*, index_aggregates.index_total_pages, index_aggregates.index_indexed_pages,
    index_aggregates.index_review_pages, index_aggregates.index_failed_pages,
    index_aggregates.index_not_required_pages, index_aggregates.index_unresolved_pages,
    index_aggregates.index_invalid_statuses
FROM books b
LEFT JOIN index_aggregates ON index_aggregates.book_id = b.id
WHERE (? IS NULL OR b.id = ?)
  AND (? IS NULL OR (b.sha256 = ? AND b.id <> ?))
ORDER BY b.created_at DESC, b.id
"#;

fn index_aggregate_from_row(row: &SqliteRow) -> AppResult<IndexAggregate> {
    let total_pages = optional_count(row, "index_total_pages")?;
    let indexed_pages = optional_count(row, "index_indexed_pages")?;
    let review_pages = optional_count(row, "index_review_pages")?;
    let failed_pages = optional_count(row, "index_failed_pages")?;
    let not_required_pages = optional_count(row, "index_not_required_pages")?;
    let unresolved_pages = optional_count(row, "index_unresolved_pages")?;
    let invalid_statuses = optional_count(row, "index_invalid_statuses")?;

    if invalid_statuses != 0 {
        return Err(AppError::new(AppErrorCode::DatabaseError));
    }

    let accounted_pages = indexed_pages
        .checked_add(review_pages)
        .and_then(|total| total.checked_add(failed_pages))
        .and_then(|total| total.checked_add(not_required_pages))
        .and_then(|total| total.checked_add(unresolved_pages))
        .ok_or_else(|| AppError::new(AppErrorCode::DatabaseError))?;
    if accounted_pages != total_pages {
        return Err(AppError::new(AppErrorCode::DatabaseError));
    }

    let status = if total_pages == 0 {
        BookIndexAggregateStatus::NotRequired
    } else if unresolved_pages > 0 {
        BookIndexAggregateStatus::Partial
    } else if failed_pages > 0 {
        if indexed_pages + not_required_pages > 0 || review_pages > 0 {
            BookIndexAggregateStatus::Partial
        } else {
            BookIndexAggregateStatus::Failed
        }
    } else if review_pages > 0 {
        BookIndexAggregateStatus::NeedsReview
    } else {
        BookIndexAggregateStatus::Ready
    };

    Ok(IndexAggregate {
        status,
        total_pages,
        indexed_pages,
        review_pages,
        failed_pages,
    })
}

fn optional_count(row: &SqliteRow, column: &str) -> AppResult<u32> {
    let value = row.try_get::<Option<i64>, _>(column)?.unwrap_or(0);
    u32::try_from(value).map_err(|_| AppError::new(AppErrorCode::DatabaseError))
}

fn parse_datetime(value: String) -> AppResult<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(&value)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|_| AppError::new(AppErrorCode::DatabaseError))
}

fn parse_format(value: &str) -> AppResult<BookFormat> {
    match value {
        "pdf" => Ok(BookFormat::Pdf),
        "epub" => Ok(BookFormat::Epub),
        "docx" => Ok(BookFormat::Docx),
        _ => Err(AppError::new(AppErrorCode::DatabaseError)),
    }
}

fn parse_status(value: &str) -> AppResult<ImportStatus> {
    match value {
        "queued" => Ok(ImportStatus::Queued),
        "copying" => Ok(ImportStatus::Copying),
        "parsing" => Ok(ImportStatus::Parsing),
        "indexing" => Ok(ImportStatus::Indexing),
        "ready" => Ok(ImportStatus::Ready),
        "failed" => Ok(ImportStatus::Failed),
        _ => Err(AppError::new(AppErrorCode::DatabaseError)),
    }
}

fn parse_error_stage(value: &str) -> AppResult<ImportErrorStage> {
    match value {
        "copying" => Ok(ImportErrorStage::Copying),
        "parsing" => Ok(ImportErrorStage::Parsing),
        "indexing" => Ok(ImportErrorStage::Indexing),
        _ => Err(AppError::new(AppErrorCode::DatabaseError)),
    }
}

fn format_name(format: &BookFormat) -> &'static str {
    match format {
        BookFormat::Pdf => "pdf",
        BookFormat::Epub => "epub",
        BookFormat::Docx => "docx",
    }
}

fn stage_name(stage: &ImportErrorStage) -> &'static str {
    match stage {
        ImportErrorStage::Copying => "copying",
        ImportErrorStage::Parsing => "parsing",
        ImportErrorStage::Indexing => "indexing",
    }
}

fn is_sha256_unique_violation(error: &sqlx::Error) -> bool {
    match error {
        sqlx::Error::Database(error) => {
            error.is_unique_violation()
                && error
                    .message()
                    .contains("UNIQUE constraint failed: books.sha256")
        }
        _ => false,
    }
}

pub fn code_name(code: AppErrorCode) -> String {
    serde_json::to_value(code)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| "LOCAL_IO_ERROR".to_owned())
}

fn ensure_changed(rows_affected: u64) -> AppResult<()> {
    if rows_affected == 0 {
        Err(AppError::new(AppErrorCode::RequestConflict))
    } else {
        Ok(())
    }
}
