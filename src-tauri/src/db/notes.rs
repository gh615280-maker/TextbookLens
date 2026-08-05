use std::fmt;

use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::{Row, Sqlite, SqlitePool, Transaction, sqlite::SqliteRow};
use uuid::Uuid;

use crate::{
    domain::ContentAnchor,
    errors::{AppError, AppErrorCode, AppResult},
};

use super::{
    annotations::validate_anchor_for_write,
    indexing::{database_timestamp, parse_timestamp, parse_uuid, to_u32},
};

pub const MAX_NOTE_CODE_POINTS: usize = 16_384;
const MAX_SELECTED_TEXT_CODE_POINTS: usize = 1_048_576;

#[derive(Clone)]
pub struct CreateNote {
    pub book_id: Uuid,
    pub section_id: Uuid,
    pub anchor: ContentAnchor,
    pub selected_text: Option<String>,
    pub note_text: String,
}

impl fmt::Debug for CreateNote {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CreateNote")
            .field("book_id", &self.book_id)
            .field("section_id", &self.section_id)
            .field("anchor", &"<redacted>")
            .field("selected_text", &"<redacted>")
            .field("note_text", &"<redacted>")
            .finish()
    }
}

#[derive(Clone)]
pub struct UpdateNote {
    pub book_id: Uuid,
    pub note_id: Uuid,
    pub expected_revision: u32,
    pub note_text: String,
}

impl fmt::Debug for UpdateNote {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UpdateNote")
            .field("book_id", &self.book_id)
            .field("note_id", &self.note_id)
            .field("expected_revision", &self.expected_revision)
            .field("note_text", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, Copy, Debug)]
pub struct DeleteNote {
    pub book_id: Uuid,
    pub note_id: Uuid,
    pub expected_revision: u32,
}

#[derive(Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteDto {
    pub id: Uuid,
    pub book_id: Uuid,
    pub section_id: Uuid,
    pub anchor: ContentAnchor,
    pub selected_text: Option<String>,
    pub note_text: String,
    pub revision: u32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl fmt::Debug for NoteDto {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NoteDto")
            .field("id", &self.id)
            .field("book_id", &self.book_id)
            .field("section_id", &self.section_id)
            .field("anchor", &"<redacted>")
            .field("selected_text", &"<redacted>")
            .field("note_text", &"<redacted>")
            .field("revision", &self.revision)
            .field("created_at", &self.created_at)
            .field("updated_at", &self.updated_at)
            .finish()
    }
}

pub async fn create_note(pool: &SqlitePool, input: CreateNote) -> AppResult<NoteDto> {
    let note_text = normalize_note_text(&input.note_text)?;
    validate_selected_text(&input.anchor, input.selected_text.as_deref())?;
    validate_anchor_for_write(pool, input.book_id, input.section_id, &input.anchor).await?;
    let anchor_json = serde_json::to_string(&input.anchor)
        .map_err(|_| AppError::new(AppErrorCode::DatabaseError))?;
    let note_id = Uuid::new_v4();
    let timestamp = database_timestamp(Utc::now());
    let mut transaction = pool.begin().await?;
    require_ready_book_and_section(&mut transaction, input.book_id, input.section_id).await?;
    sqlx::query(
        "INSERT INTO annotations (id, book_id, section_id, kind, anchor_json, selected_text, note_text, revision, created_at, updated_at) VALUES (?, ?, ?, 'note', ?, ?, ?, 1, ?, ?)",
    )
    .bind(note_id.to_string())
    .bind(input.book_id.to_string())
    .bind(input.section_id.to_string())
    .bind(anchor_json)
    .bind(input.selected_text)
    .bind(note_text)
    .bind(&timestamp)
    .bind(&timestamp)
    .execute(&mut *transaction)
    .await?;
    let note = get_note_in_transaction(&mut transaction, input.book_id, note_id).await?;
    transaction.commit().await?;
    Ok(note)
}

pub async fn update_note(pool: &SqlitePool, input: UpdateNote) -> AppResult<NoteDto> {
    if input.expected_revision == 0 {
        return Err(AppError::new(AppErrorCode::InvalidInput));
    }
    let note_text = normalize_note_text(&input.note_text)?;
    let timestamp = database_timestamp(Utc::now());
    let mut transaction = pool.begin().await?;
    require_ready_book(&mut transaction, input.book_id).await?;
    let result = sqlx::query(
        "UPDATE annotations SET note_text = ?, revision = revision + 1, updated_at = ? WHERE id = ? AND book_id = ? AND kind = 'note' AND revision = ? AND revision < 1000000",
    )
    .bind(note_text)
    .bind(timestamp)
    .bind(input.note_id.to_string())
    .bind(input.book_id.to_string())
    .bind(i64::from(input.expected_revision))
    .execute(&mut *transaction)
    .await?;
    if result.rows_affected() != 1 {
        return Err(classify_note_miss(
            &mut transaction,
            input.book_id,
            input.note_id,
            input.expected_revision,
        )
        .await?);
    }
    let note = get_note_in_transaction(&mut transaction, input.book_id, input.note_id).await?;
    transaction.commit().await?;
    Ok(note)
}

pub async fn delete_note(pool: &SqlitePool, input: DeleteNote) -> AppResult<()> {
    if input.expected_revision == 0 {
        return Err(AppError::new(AppErrorCode::InvalidInput));
    }
    let mut transaction = pool.begin().await?;
    require_ready_book(&mut transaction, input.book_id).await?;
    let result = sqlx::query(
        "DELETE FROM annotations WHERE id = ? AND book_id = ? AND kind = 'note' AND revision = ?",
    )
    .bind(input.note_id.to_string())
    .bind(input.book_id.to_string())
    .bind(i64::from(input.expected_revision))
    .execute(&mut *transaction)
    .await?;
    if result.rows_affected() != 1 {
        return Err(classify_note_miss(
            &mut transaction,
            input.book_id,
            input.note_id,
            input.expected_revision,
        )
        .await?);
    }
    transaction.commit().await?;
    Ok(())
}

pub async fn get_note(pool: &SqlitePool, book_id: Uuid, note_id: Uuid) -> AppResult<NoteDto> {
    let mut transaction = pool.begin().await?;
    require_ready_book(&mut transaction, book_id).await?;
    let note = get_note_in_transaction(&mut transaction, book_id, note_id).await?;
    transaction.commit().await?;
    Ok(note)
}

pub async fn list_notes(pool: &SqlitePool, book_id: Uuid) -> AppResult<Vec<NoteDto>> {
    let mut transaction = pool.begin().await?;
    require_ready_book(&mut transaction, book_id).await?;
    let rows = sqlx::query(
        "SELECT id, book_id, section_id, anchor_json, selected_text, note_text, revision, created_at, updated_at FROM annotations WHERE book_id = ? AND kind = 'note' ORDER BY created_at, id",
    )
    .bind(book_id.to_string())
    .fetch_all(&mut *transaction)
    .await?;
    let notes = rows
        .iter()
        .map(note_from_row)
        .collect::<AppResult<Vec<_>>>()?;
    transaction.commit().await?;
    Ok(notes)
}

fn normalize_note_text(value: &str) -> AppResult<String> {
    let normalized = value.replace("\r\n", "\n").replace('\r', "\n");
    let code_points = normalized.chars().count();
    if normalized.trim().is_empty() || code_points > MAX_NOTE_CODE_POINTS {
        return Err(AppError::new(AppErrorCode::InvalidInput));
    }
    Ok(normalized)
}

fn validate_selected_text(anchor: &ContentAnchor, selected_text: Option<&str>) -> AppResult<()> {
    let expected = match anchor {
        ContentAnchor::Text { selection } => Some(selection.quote.exact.as_str()),
        ContentAnchor::Region { region } => region
            .text_fallback
            .as_ref()
            .map(|quote| quote.exact.as_str()),
    };
    if selected_text != expected
        || selected_text.is_some_and(|value| value.chars().count() > MAX_SELECTED_TEXT_CODE_POINTS)
    {
        return Err(AppError::new(AppErrorCode::InvalidInput));
    }
    Ok(())
}

async fn require_ready_book(
    transaction: &mut Transaction<'_, Sqlite>,
    book_id: Uuid,
) -> AppResult<()> {
    let status = sqlx::query_scalar::<_, String>("SELECT import_status FROM books WHERE id = ?")
        .bind(book_id.to_string())
        .fetch_optional(&mut **transaction)
        .await?
        .ok_or_else(|| AppError::new(AppErrorCode::NotFound))?;
    if status != "ready" {
        return Err(AppError::new(AppErrorCode::BookNotReady));
    }
    Ok(())
}

async fn require_ready_book_and_section(
    transaction: &mut Transaction<'_, Sqlite>,
    book_id: Uuid,
    section_id: Uuid,
) -> AppResult<()> {
    require_ready_book(transaction, book_id).await?;
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM sections WHERE id = ? AND book_id = ?)")
            .bind(section_id.to_string())
            .bind(book_id.to_string())
            .fetch_one(&mut **transaction)
            .await?;
    if !exists {
        return Err(AppError::new(AppErrorCode::AnchorNotFound));
    }
    Ok(())
}

async fn get_note_in_transaction(
    transaction: &mut Transaction<'_, Sqlite>,
    book_id: Uuid,
    note_id: Uuid,
) -> AppResult<NoteDto> {
    let row = sqlx::query(
        "SELECT id, book_id, section_id, anchor_json, selected_text, note_text, revision, created_at, updated_at FROM annotations WHERE id = ? AND book_id = ? AND kind = 'note'",
    )
    .bind(note_id.to_string())
    .bind(book_id.to_string())
    .fetch_optional(&mut **transaction)
    .await?
    .ok_or_else(|| AppError::new(AppErrorCode::NotFound))?;
    note_from_row(&row)
}

async fn classify_note_miss(
    transaction: &mut Transaction<'_, Sqlite>,
    book_id: Uuid,
    note_id: Uuid,
    expected_revision: u32,
) -> AppResult<AppError> {
    let row = sqlx::query("SELECT book_id, kind, revision FROM annotations WHERE id = ?")
        .bind(note_id.to_string())
        .fetch_optional(&mut **transaction)
        .await?;
    let Some(row) = row else {
        return Ok(AppError::new(AppErrorCode::NotFound));
    };
    if row.try_get::<String, _>("book_id")? != book_id.to_string()
        || row.try_get::<String, _>("kind")? != "note"
    {
        return Ok(AppError::new(AppErrorCode::NotFound));
    }
    let revision = to_u32(row.try_get::<i64, _>("revision")?)?;
    if revision != expected_revision || revision >= 1_000_000 {
        return Ok(AppError::new(AppErrorCode::RequestConflict));
    }
    Ok(AppError::new(AppErrorCode::RequestConflict))
}

fn note_from_row(row: &SqliteRow) -> AppResult<NoteDto> {
    let anchor_json: String = row.try_get("anchor_json")?;
    let anchor = serde_json::from_str(&anchor_json)
        .map_err(|_| AppError::new(AppErrorCode::DatabaseError))?;
    Ok(NoteDto {
        id: parse_uuid(row.try_get("id")?)?,
        book_id: parse_uuid(row.try_get("book_id")?)?,
        section_id: parse_uuid(row.try_get("section_id")?)?,
        anchor,
        selected_text: row.try_get("selected_text")?,
        note_text: row.try_get("note_text")?,
        revision: to_u32(row.try_get("revision")?)?,
        created_at: parse_timestamp(&row.try_get::<String, _>("created_at")?)?,
        updated_at: parse_timestamp(&row.try_get::<String, _>("updated_at")?)?,
    })
}

#[cfg(test)]
#[path = "notes_test.rs"]
mod tests;
