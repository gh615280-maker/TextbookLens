use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::{
    domain::{
        AnnotationKind, ContentAnchor, DocumentLocator, RegionAnchor, RegionLocator,
        SelectionAnchor, TextQuote,
    },
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
    pub anchor: Option<ContentAnchor>,
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
            .and_then(|json| serde_json::from_str::<ContentAnchor>(&json).ok());
        let (anchor, relocation_status) = match candidate {
            Some(anchor) => {
                match resolve_anchor(pool, book_id, &format, section_id, &anchor).await? {
                    Some(status) => (Some(anchor), status),
                    None => (None, MarkerRelocationStatus::Unresolved),
                }
            }
            None => (None, MarkerRelocationStatus::Unresolved),
        };
        markers.push(AnnotationMarkerDto {
            id,
            kind,
            relocation_status,
            anchor,
        });
    }
    Ok(markers)
}

async fn resolve_anchor(
    pool: &SqlitePool,
    book_id: Uuid,
    format: &str,
    annotation_section_id: Option<Uuid>,
    anchor: &ContentAnchor,
) -> AppResult<Option<MarkerRelocationStatus>> {
    match anchor {
        ContentAnchor::Text { selection } => {
            Ok(
                valid_selection_anchor(pool, book_id, format, annotation_section_id, selection)
                    .await?
                    .then_some(MarkerRelocationStatus::Primary),
            )
        }
        ContentAnchor::Region { region } => {
            resolve_region_anchor(pool, book_id, format, annotation_section_id, region).await
        }
    }
}

pub(crate) async fn validate_anchor_for_write(
    pool: &SqlitePool,
    book_id: Uuid,
    section_id: Uuid,
    anchor: &ContentAnchor,
) -> AppResult<()> {
    let book = sqlx::query("SELECT format, import_status FROM books WHERE id = ?")
        .bind(book_id.to_string())
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| AppError::new(AppErrorCode::NotFound))?;
    if book.try_get::<String, _>("import_status")? != "ready" {
        return Err(AppError::new(AppErrorCode::BookNotReady));
    }
    let format: String = book.try_get("format")?;
    match resolve_anchor(pool, book_id, &format, Some(section_id), anchor).await? {
        Some(MarkerRelocationStatus::Primary) => Ok(()),
        Some(MarkerRelocationStatus::Fallback)
        | Some(MarkerRelocationStatus::Unresolved)
        | None => Err(AppError::new(AppErrorCode::AnchorNotFound)),
    }
}

async fn valid_selection_anchor(
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

async fn resolve_region_anchor(
    pool: &SqlitePool,
    book_id: Uuid,
    format: &str,
    annotation_section_id: Option<Uuid>,
    anchor: &RegionAnchor,
) -> AppResult<Option<MarkerRelocationStatus>> {
    let Some(section_id) = annotation_section_id else {
        return Ok(None);
    };
    let Some(section_locator_json) = sqlx::query_scalar::<_, String>(
        "SELECT locator_json FROM sections WHERE id = ? AND book_id = ?",
    )
    .bind(section_id.to_string())
    .bind(book_id.to_string())
    .fetch_optional(pool)
    .await?
    else {
        return Ok(None);
    };
    let section_locator = serde_json::from_str::<DocumentLocator>(&section_locator_json).ok();

    let primary = match (&anchor.locator, format, section_locator.as_ref()) {
        (
            RegionLocator::Pdf { page },
            "pdf",
            Some(DocumentLocator::Pdf {
                start_page,
                end_page,
                ..
            }),
        ) => page >= start_page && page <= end_page,
        (
            RegionLocator::Epub {
                section_id: locator_section,
                cfi,
            },
            "epub",
            Some(DocumentLocator::Epub {
                section_id: stored_section,
                cfi: stored_cfi,
            }),
        ) => {
            locator_section == &section_id
                && stored_section == &section_id
                && (cfi == stored_cfi
                    || epub_block_locator_exists(pool, book_id, section_id, cfi).await?)
        }
        (RegionLocator::Docx { block_id }, "docx", _) => {
            block_info(pool, book_id, section_id, *block_id)
                .await?
                .is_some()
        }
        _ => return Ok(None),
    };

    if primary && primary_hash_semantics_hold(pool, book_id, section_id, anchor).await? {
        return Ok(Some(MarkerRelocationStatus::Primary));
    }

    let Some(fallback) = anchor.text_fallback.as_ref() else {
        return Ok(None);
    };
    if sha256_text(&fallback.exact) != anchor.content_sha256 {
        return Ok(None);
    }
    let matches =
        fallback_match_count(pool, book_id, section_id, &anchor.locator, fallback).await?;
    Ok((matches == 1).then_some(MarkerRelocationStatus::Fallback))
}

async fn epub_block_locator_exists(
    pool: &SqlitePool,
    book_id: Uuid,
    section_id: Uuid,
    cfi: &str,
) -> AppResult<bool> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM blocks WHERE book_id = ? AND section_id = ? AND json_extract(locator_json, '$.format') = 'epub' AND json_extract(locator_json, '$.sectionId') = ? AND json_extract(locator_json, '$.cfi') = ?)",
    )
    .bind(book_id.to_string())
    .bind(section_id.to_string())
    .bind(section_id.to_string())
    .bind(cfi)
    .fetch_one(pool)
    .await?)
}

async fn primary_hash_semantics_hold(
    pool: &SqlitePool,
    book_id: Uuid,
    section_id: Uuid,
    anchor: &RegionAnchor,
) -> AppResult<bool> {
    let Some(fallback) = anchor.text_fallback.as_ref() else {
        // Visual regions use a hash of bounded pixels that can only be reverified by the
        // reader adapter. The database validates its strict digest shape and ownership.
        return Ok(true);
    };
    if sha256_text(&fallback.exact) != anchor.content_sha256 {
        // A visual region may retain text as a fallback while hashing pixels.
        return Ok(true);
    }
    Ok(fallback_match_count(pool, book_id, section_id, &anchor.locator, fallback).await? > 0)
}

async fn fallback_match_count(
    pool: &SqlitePool,
    book_id: Uuid,
    section_id: Uuid,
    locator: &RegionLocator,
    fallback: &TextQuote,
) -> AppResult<usize> {
    let rows = sqlx::query(
        "SELECT id, plain_text, locator_json FROM blocks WHERE book_id = ? AND section_id = ? ORDER BY ordinal",
    )
    .bind(book_id.to_string())
    .bind(section_id.to_string())
    .fetch_all(pool)
    .await?;
    let mut count = 0usize;
    for row in rows {
        let id = parse_uuid(&row.try_get::<String, _>("id")?)?;
        let text: String = row.try_get("plain_text")?;
        let locator_json: String = row.try_get("locator_json")?;
        let block_locator = serde_json::from_str::<DocumentLocator>(&locator_json).ok();
        let same_container = match (locator, block_locator.as_ref()) {
            (
                RegionLocator::Pdf { page },
                Some(DocumentLocator::Pdf {
                    start_page,
                    end_page,
                    ..
                }),
            ) => page >= start_page && page <= end_page,
            (
                RegionLocator::Epub {
                    section_id: owner, ..
                },
                _,
            ) => owner == &section_id,
            (RegionLocator::Docx { block_id }, _) => block_id == &id,
            _ => false,
        };
        if same_container {
            count = count.saturating_add(text.match_indices(&fallback.exact).count());
            if count > 1 {
                return Ok(count);
            }
        }
    }
    Ok(count)
}

fn sha256_text(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
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
