use serde::Serialize;
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::{
    domain::{AnnotationKind, DocumentLocator, SelectionAnchor},
    errors::{AppError, AppErrorCode, AppResult},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MarkerRelocationStatus {
    Primary,
    Fallback,
    Unresolved,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnnotationMarkerDto {
    pub id: Uuid,
    pub kind: AnnotationKind,
    pub anchor: Option<SelectionAnchor>,
    pub relocation_status: MarkerRelocationStatus,
}

pub async fn list_annotation_markers(
    pool: &SqlitePool,
    book_id: Uuid,
) -> AppResult<Vec<AnnotationMarkerDto>> {
    let book = sqlx::query("SELECT format, import_status FROM books WHERE id = ?")
        .bind(book_id.to_string())
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| AppError::new(AppErrorCode::NotFound))?;
    if book.try_get::<String, _>("import_status")? != "ready" {
        return Err(AppError::new(AppErrorCode::BookNotReady));
    }
    let format: String = book.try_get("format")?;
    let rows = sqlx::query(
        "SELECT id, kind, section_id, anchor_json FROM annotations WHERE book_id = ? ORDER BY created_at, id",
    )
    .bind(book_id.to_string())
    .fetch_all(pool)
    .await?;

    let mut markers = Vec::with_capacity(rows.len());
    for row in rows {
        let id = parse_uuid(&row.try_get::<String, _>("id")?)?;
        let kind = match row.try_get::<String, _>("kind")?.as_str() {
            "ai_conversation" => AnnotationKind::AiConversation,
            "note" => AnnotationKind::Note,
            _ => return Err(AppError::new(AppErrorCode::DatabaseError)),
        };
        let section_id = row
            .try_get::<Option<String>, _>("section_id")?
            .map(|value| parse_uuid(&value))
            .transpose()?;
        let candidate = row
            .try_get::<Option<String>, _>("anchor_json")?
            .and_then(|json| serde_json::from_str::<SelectionAnchor>(&json).ok());
        let anchor = match candidate {
            Some(anchor) if valid_anchor(pool, book_id, &format, section_id, &anchor).await? => {
                Some(anchor)
            }
            _ => None,
        };
        markers.push(AnnotationMarkerDto {
            id,
            kind,
            relocation_status: if anchor.is_some() {
                MarkerRelocationStatus::Primary
            } else {
                MarkerRelocationStatus::Unresolved
            },
            anchor,
        });
    }
    Ok(markers)
}

async fn valid_anchor(
    pool: &SqlitePool,
    book_id: Uuid,
    format: &str,
    annotation_section_id: Option<Uuid>,
    anchor: &SelectionAnchor,
) -> AppResult<bool> {
    if anchor.quote.exact.is_empty()
        || anchor.quote.prefix.chars().count() > 64
        || anchor.quote.suffix.chars().count() > 64
        || anchor.section_id != annotation_section_id
    {
        return Ok(false);
    }
    let Some(section_id) = anchor.section_id else {
        return Ok(false);
    };
    let section_exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM sections WHERE id = ? AND book_id = ?)")
            .bind(section_id.to_string())
            .bind(book_id.to_string())
            .fetch_one(pool)
            .await?;
    if !section_exists {
        return Ok(false);
    }
    match (&anchor.locator, format) {
        (
            DocumentLocator::Pdf {
                start_page,
                end_page,
                rects_by_page,
            },
            "pdf",
        ) => {
            if *start_page == 0 || start_page > end_page {
                return Ok(false);
            }
            Ok(rects_by_page.as_ref().is_none_or(|pages| {
                pages.iter().all(|(page, rects)| {
                    *page >= *start_page
                        && *page <= *end_page
                        && rects.iter().all(|rect| {
                            [rect.x, rect.y, rect.width, rect.height]
                                .iter()
                                .all(|value| value.is_finite() && (0.0..=1.0).contains(value))
                                && rect.x + rect.width <= 1.0
                                && rect.y + rect.height <= 1.0
                        })
                })
            }))
        }
        (
            DocumentLocator::Epub {
                cfi,
                section_id: locator_section,
            },
            "epub",
        ) => Ok(!cfi.trim().is_empty() && *locator_section == section_id),
        (
            DocumentLocator::Docx {
                start_block_id,
                start_offset,
                end_block_id,
                end_offset,
            },
            "docx",
        ) => {
            let start = block_info(pool, book_id, section_id, *start_block_id).await?;
            let end = block_info(pool, book_id, section_id, *end_block_id).await?;
            Ok(
                matches!((start, end), (Some((start_ordinal, start_len)), Some((end_ordinal, end_len))) if start_ordinal <= end_ordinal && *start_offset <= start_len && *end_offset <= end_len && (start_ordinal != end_ordinal || start_offset <= end_offset)),
            )
        }
        _ => Ok(false),
    }
}

async fn block_info(
    pool: &SqlitePool,
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
    .fetch_optional(pool)
    .await?;
    row.map(|row| {
        let ordinal = row.try_get("ordinal")?;
        let text: String = row.try_get("plain_text")?;
        let length = u32::try_from(text.chars().count())
            .map_err(|_| AppError::new(AppErrorCode::DatabaseError))?;
        Ok((ordinal, length))
    })
    .transpose()
}

fn parse_uuid(value: &str) -> AppResult<Uuid> {
    Uuid::parse_str(value).map_err(|_| AppError::new(AppErrorCode::DatabaseError))
}
