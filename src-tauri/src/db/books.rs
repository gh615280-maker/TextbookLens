use chrono::{DateTime, Utc};
use sqlx::{Row, SqlitePool, sqlite::SqliteRow};
use uuid::Uuid;

use crate::{
    domain::{BookFormat, BookSummary, ImportStatus},
    errors::{AppError, AppErrorCode, AppResult},
};

#[derive(Clone, Debug)]
pub struct BookRecord {
    pub summary: BookSummary,
    pub sha256: String,
    pub stored_path: String,
}

#[derive(Clone, Debug)]
pub enum HashClaim {
    Claimed(BookSummary),
    Duplicate(BookSummary),
}

pub async fn insert_copying(
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
        "INSERT INTO books (id, sha256, title, format, original_filename, stored_path, import_status, created_at, updated_at) VALUES (?, ?, ?, ?, ?, '', 'copying', strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
    )
    .bind(id.to_string())
    .bind(format!("pending:{id}"))
    .bind(title)
    .bind(format_name(format))
    .bind(original_filename)
    .execute(pool)
    .await?;
    Ok(())
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
            "UPDATE books SET sha256 = ?, stored_path = ?, original_filename = ?, import_status = 'parsing', import_error_code = NULL, import_error_message = NULL, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?",
        )
        .bind(sha256)
        .bind(stored_path)
        .bind(original_filename)
        .bind(id.to_string())
        .execute(&mut *transaction)
        .await
    } else {
        sqlx::query(
            "UPDATE books SET sha256 = ?, stored_path = ?, import_status = 'parsing', import_error_code = NULL, import_error_message = NULL, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?",
        )
        .bind(sha256)
        .bind(stored_path)
        .bind(id.to_string())
        .execute(&mut *transaction)
        .await
    };

    if let Err(error) = updated {
        let duplicate = sqlx::query("SELECT * FROM books WHERE sha256 = ? AND id <> ?")
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

    let book = fetch_in_transaction(&mut transaction, id).await?;
    transaction.commit().await?;
    Ok(HashClaim::Claimed(book.summary))
}

pub async fn get(pool: &SqlitePool, id: Uuid) -> AppResult<BookRecord> {
    let row = sqlx::query("SELECT * FROM books WHERE id = ?")
        .bind(id.to_string())
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| AppError::new(AppErrorCode::NotFound))?;
    row_to_record(&row)
}

pub async fn list(pool: &SqlitePool) -> AppResult<Vec<BookSummary>> {
    sqlx::query("SELECT * FROM books ORDER BY created_at DESC, id")
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
    code: AppErrorCode,
    message: &str,
    clear_stored_path: bool,
) -> AppResult<()> {
    let result = if clear_stored_path {
        sqlx::query(
            "UPDATE books SET import_status = 'failed', import_error_code = ?, import_error_message = ?, stored_path = '', updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?",
        )
        .bind(code_name(code))
        .bind(message)
        .bind(id.to_string())
        .execute(pool)
        .await?
    } else {
        sqlx::query(
            "UPDATE books SET import_status = 'failed', import_error_code = ?, import_error_message = ?, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?",
        )
        .bind(code_name(code))
        .bind(message)
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
        "UPDATE books SET title = ?, author = ?, language = ?, import_status = 'parsing', import_error_code = NULL, import_error_message = NULL, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ? AND import_status = 'parsing'",
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
        "UPDATE books SET import_status = 'ready', import_error_code = NULL, import_error_message = NULL, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ? AND import_status = 'parsing'",
    )
    .bind(id.to_string())
    .execute(pool)
    .await?;
    ensure_changed(result.rows_affected())?;
    Ok(get(pool, id).await?.summary)
}

pub async fn reset_failed_for_retry(pool: &SqlitePool, id: Uuid) -> AppResult<BookSummary> {
    let result = sqlx::query(
        "UPDATE books SET import_status = 'parsing', import_error_code = NULL, import_error_message = NULL, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ? AND import_status = 'failed'",
    )
    .bind(id.to_string())
    .execute(pool)
    .await?;
    ensure_changed(result.rows_affected())?;
    Ok(get(pool, id).await?.summary)
}

pub async fn prepare_failed_for_replacement(pool: &SqlitePool, id: Uuid) -> AppResult<()> {
    let result = sqlx::query(
        "UPDATE books SET sha256 = ?, stored_path = '', import_status = 'copying', import_error_code = NULL, import_error_message = NULL, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ? AND import_status = 'failed'",
    )
    .bind(format!("pending:{id}"))
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
    let row = sqlx::query("SELECT * FROM books WHERE id = ?")
        .bind(id.to_string())
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
    Ok(BookRecord {
        sha256: row.try_get("sha256")?,
        stored_path: row.try_get("stored_path")?,
        summary: BookSummary {
            id,
            title: row.try_get("title")?,
            author: row.try_get("author")?,
            language: row.try_get("language")?,
            format,
            import_status,
            import_error_code: row.try_get("import_error_code")?,
            import_error_message: row.try_get("import_error_message")?,
            reading_progress: row.try_get("reading_progress")?,
            created_at,
            updated_at,
            last_opened_at,
        },
    })
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
        "copying" => Ok(ImportStatus::Copying),
        "parsing" => Ok(ImportStatus::Parsing),
        "indexing" => Ok(ImportStatus::Indexing),
        "ready" => Ok(ImportStatus::Ready),
        "failed" => Ok(ImportStatus::Failed),
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
