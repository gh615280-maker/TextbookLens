use std::{collections::BTreeMap, fmt};

use chrono::Utc;
use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::{Row, Sqlite, SqlitePool, Transaction};
use uuid::Uuid;

use crate::{
    domain::{
        AnnotationKind, ContentAnchor, DocumentLocator, RegionAnchor, RegionLocator,
        SelectionAnchor, TextQuote,
    },
    errors::{AppError, AppErrorCode, AppResult},
};

use super::indexing::database_timestamp;

const MAX_SUMMARY_CODE_POINTS: usize = 512;
const DEFAULT_SUMMARY_CODE_POINTS: usize = 120;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MarkerRelocationStatus {
    Primary,
    Fallback,
    Unresolved,
}

#[derive(Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnnotationMarkerDto {
    pub id: Uuid,
    pub kind: AnnotationKind,
    pub conversation_id: Option<Uuid>,
    pub anchor: Option<ContentAnchor>,
    pub relocation_status: MarkerRelocationStatus,
    pub accessibility_label: &'static str,
    pub sequence: Option<u32>,
    pub summary_text: String,
    pub revision: u32,
}

impl fmt::Debug for AnnotationMarkerDto {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AnnotationMarkerDto")
            .field("id", &"<redacted>")
            .field("kind", &self.kind)
            .field(
                "conversation_id",
                &self.conversation_id.map(|_| "<redacted>"),
            )
            .field("anchor", &"<redacted>")
            .field("relocation_status", &self.relocation_status)
            .field("accessibility_label", &self.accessibility_label)
            .field("sequence", &self.sequence)
            .field("summary_text", &"<redacted>")
            .field("revision", &self.revision)
            .finish()
    }
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
    let relocation = MarkerRelocationContext::load(pool, book_id).await?;
    let rows = sqlx::query(
        "SELECT annotation.id, annotation.kind, annotation.section_id, annotation.anchor_json, annotation.conversation_id, annotation.selected_text, annotation.note_text, annotation.summary_text, annotation.revision, (SELECT message.content FROM messages message WHERE message.conversation_id = annotation.conversation_id AND message.role = 'assistant' ORDER BY message.ordinal LIMIT 1) AS assistant_content FROM annotations annotation WHERE annotation.book_id = ? ORDER BY annotation.created_at, annotation.id",
    )
    .bind(book_id.to_string())
    .fetch_all(pool)
    .await?;

    let mut markers = Vec::with_capacity(rows.len());
    let mut ai_sequence = 0u32;
    for row in rows {
        let id = parse_uuid(&row.try_get::<String, _>("id")?)?;
        let (kind, accessibility_label) = match row.try_get::<String, _>("kind")?.as_str() {
            "ai_conversation" => (
                AnnotationKind::AiConversation,
                "View AI conversation marker",
            ),
            "note" => (AnnotationKind::Note, "View personal note marker"),
            _ => return Err(AppError::new(AppErrorCode::DatabaseError)),
        };
        let conversation_id = row
            .try_get::<Option<String>, _>("conversation_id")?
            .map(|value| parse_uuid(&value))
            .transpose()?;
        if matches!(kind, AnnotationKind::AiConversation) != conversation_id.is_some() {
            return Err(AppError::new(AppErrorCode::DatabaseError));
        }
        let section_id = row
            .try_get::<Option<String>, _>("section_id")?
            .map(|value| parse_uuid(&value))
            .transpose()?;
        let anchor = row
            .try_get::<Option<String>, _>("anchor_json")?
            .and_then(|json| serde_json::from_str::<ContentAnchor>(&json).ok());
        let relocation_status = anchor
            .as_ref()
            .and_then(|anchor| relocation.resolve(&format, section_id, anchor))
            .unwrap_or(MarkerRelocationStatus::Unresolved);
        let sequence = if matches!(kind, AnnotationKind::AiConversation) {
            ai_sequence = ai_sequence
                .checked_add(1)
                .ok_or_else(|| AppError::new(AppErrorCode::DatabaseError))?;
            Some(ai_sequence)
        } else {
            None
        };
        let stored_summary = row.try_get::<Option<String>, _>("summary_text")?;
        let assistant_content = row.try_get::<Option<String>, _>("assistant_content")?;
        let selected_text = row.try_get::<Option<String>, _>("selected_text")?;
        let note_text = row.try_get::<Option<String>, _>("note_text")?;
        let summary_text = stored_summary.unwrap_or_else(|| {
            default_summary(
                assistant_content
                    .as_deref()
                    .or(selected_text.as_deref())
                    .or(note_text.as_deref())
                    .unwrap_or("Question"),
            )
        });
        let revision = u32::try_from(row.try_get::<i64, _>("revision")?)
            .map_err(|_| AppError::new(AppErrorCode::DatabaseError))?;
        markers.push(AnnotationMarkerDto {
            id,
            kind,
            conversation_id,
            relocation_status,
            anchor,
            accessibility_label,
            sequence,
            summary_text,
            revision,
        });
    }
    Ok(markers)
}

pub async fn update_ai_annotation_summary(
    pool: &SqlitePool,
    book_id: Uuid,
    annotation_id: Uuid,
    expected_revision: u32,
    summary_text: String,
) -> AppResult<()> {
    if expected_revision == 0 {
        return Err(AppError::new(AppErrorCode::InvalidInput));
    }
    let summary_text = normalize_summary(&summary_text)?;
    let timestamp = database_timestamp(Utc::now());
    let result = sqlx::query(
        "UPDATE annotations SET summary_text = ?, revision = revision + 1, updated_at = ? WHERE id = ? AND book_id = ? AND kind = 'ai_conversation' AND revision = ? AND revision < 1000000",
    )
    .bind(summary_text)
    .bind(timestamp)
    .bind(annotation_id.to_string())
    .bind(book_id.to_string())
    .bind(i64::from(expected_revision))
    .execute(pool)
    .await?;
    if result.rows_affected() != 1 {
        return Err(AppError::new(AppErrorCode::RequestConflict));
    }
    Ok(())
}

fn normalize_summary(value: &str) -> AppResult<String> {
    let normalized = value.trim().replace("\r\n", "\n");
    let count = normalized.chars().count();
    if count == 0
        || count > MAX_SUMMARY_CODE_POINTS
        || normalized.contains('\0')
        || normalized.contains('\r')
    {
        return Err(AppError::new(AppErrorCode::InvalidInput));
    }
    Ok(normalized)
}

fn default_summary(value: &str) -> String {
    let compact = value.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut summary = String::new();
    for character in compact.chars().take(DEFAULT_SUMMARY_CODE_POINTS) {
        summary.push(character);
        if matches!(character, '。' | '！' | '？' | '.' | '!' | '?') {
            break;
        }
    }
    if compact.chars().count() > summary.chars().count()
        && !summary
            .chars()
            .last()
            .is_some_and(|character| matches!(character, '。' | '！' | '？' | '.' | '!' | '?'))
    {
        summary.push('…');
    }
    if summary.is_empty() {
        "Question".to_owned()
    } else {
        summary
    }
}

struct MarkerRelocationContext {
    sections: BTreeMap<Uuid, DocumentLocator>,
    blocks: Vec<MarkerBlock>,
}

struct MarkerBlock {
    id: Uuid,
    section_id: Uuid,
    ordinal: i64,
    plain_text: String,
    locator: Option<DocumentLocator>,
}

impl MarkerRelocationContext {
    async fn load(pool: &SqlitePool, book_id: Uuid) -> AppResult<Self> {
        let section_rows = sqlx::query(
            "SELECT id, locator_json FROM sections WHERE book_id = ? ORDER BY ordinal, id",
        )
        .bind(book_id.to_string())
        .fetch_all(pool)
        .await?;
        let mut sections = BTreeMap::new();
        for row in section_rows {
            let id = parse_uuid(&row.try_get::<String, _>("id")?)?;
            let locator = serde_json::from_str(&row.try_get::<String, _>("locator_json")?)
                .map_err(|_| AppError::new(AppErrorCode::DatabaseError))?;
            if sections.insert(id, locator).is_some() {
                return Err(AppError::new(AppErrorCode::DatabaseError));
            }
        }
        let block_rows = sqlx::query(
            "SELECT id, section_id, ordinal, plain_text, locator_json FROM blocks WHERE book_id = ? ORDER BY section_id, ordinal, id",
        )
        .bind(book_id.to_string())
        .fetch_all(pool)
        .await?;
        let mut blocks = Vec::with_capacity(block_rows.len());
        for row in block_rows {
            blocks.push(MarkerBlock {
                id: parse_uuid(&row.try_get::<String, _>("id")?)?,
                section_id: parse_uuid(&row.try_get::<String, _>("section_id")?)?,
                ordinal: row.try_get("ordinal")?,
                plain_text: row.try_get("plain_text")?,
                locator: serde_json::from_str(&row.try_get::<String, _>("locator_json")?).ok(),
            });
        }
        Ok(Self { sections, blocks })
    }

    fn resolve(
        &self,
        format: &str,
        annotation_section_id: Option<Uuid>,
        anchor: &ContentAnchor,
    ) -> Option<MarkerRelocationStatus> {
        match anchor {
            ContentAnchor::Text { selection } => self
                .valid_selection(format, annotation_section_id, selection)
                .then_some(MarkerRelocationStatus::Primary),
            ContentAnchor::Region { region } => {
                self.resolve_region(format, annotation_section_id, region)
            }
        }
    }

    fn valid_selection(
        &self,
        format: &str,
        annotation_section_id: Option<Uuid>,
        anchor: &SelectionAnchor,
    ) -> bool {
        if anchor.quote.exact.is_empty()
            || anchor.quote.prefix.chars().count() > 64
            || anchor.quote.suffix.chars().count() > 64
            || anchor.section_id != annotation_section_id
        {
            return false;
        }
        let Some(section_id) = anchor.section_id else {
            return false;
        };
        if !self.sections.contains_key(&section_id) {
            return false;
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
                *start_page > 0
                    && start_page <= end_page
                    && rects_by_page.as_ref().is_none_or(|pages| {
                        pages.iter().all(|(page, rects)| {
                            *page >= *start_page
                                && *page <= *end_page
                                && rects.iter().all(valid_normalized_rect)
                        })
                    })
            }
            (
                DocumentLocator::Epub {
                    cfi,
                    section_id: locator_section,
                },
                "epub",
            ) => !cfi.trim().is_empty() && *locator_section == section_id,
            (
                DocumentLocator::Docx {
                    start_block_id,
                    start_offset,
                    end_block_id,
                    end_offset,
                },
                "docx",
            ) => {
                let start = self.block_info(section_id, *start_block_id);
                let end = self.block_info(section_id, *end_block_id);
                matches!((start, end), (Some((start_ordinal, start_len)), Some((end_ordinal, end_len))) if start_ordinal <= end_ordinal && *start_offset <= start_len && *end_offset <= end_len && (start_ordinal != end_ordinal || start_offset <= end_offset))
            }
            _ => false,
        }
    }

    fn resolve_region(
        &self,
        format: &str,
        annotation_section_id: Option<Uuid>,
        anchor: &RegionAnchor,
    ) -> Option<MarkerRelocationStatus> {
        let section_id = annotation_section_id?;
        let section_locator = self.sections.get(&section_id)?;
        let primary = match (&anchor.locator, format, section_locator) {
            (
                RegionLocator::Pdf { page },
                "pdf",
                DocumentLocator::Pdf {
                    start_page,
                    end_page,
                    ..
                },
            ) => page >= start_page && page <= end_page,
            (
                RegionLocator::Epub {
                    section_id: locator_section,
                    cfi,
                },
                "epub",
                DocumentLocator::Epub {
                    section_id: stored_section,
                    cfi: stored_cfi,
                },
            ) => {
                locator_section == &section_id
                    && stored_section == &section_id
                    && (cfi == stored_cfi
                        || self.blocks.iter().any(|block| {
                            block.section_id == section_id
                                && matches!(&block.locator, Some(DocumentLocator::Epub { section_id: block_section, cfi: block_cfi }) if block_section == &section_id && block_cfi == cfi)
                        }))
            }
            (RegionLocator::Docx { block_id }, "docx", _) => {
                self.block_info(section_id, *block_id).is_some()
            }
            _ => false,
        };
        if primary && self.primary_hash_semantics_hold(section_id, anchor) {
            return Some(MarkerRelocationStatus::Primary);
        }
        let fallback = anchor.text_fallback.as_ref()?;
        if sha256_text(&fallback.exact) != anchor.content_sha256 {
            return None;
        }
        (self.fallback_match_count(section_id, &anchor.locator, fallback) == 1)
            .then_some(MarkerRelocationStatus::Fallback)
    }

    fn primary_hash_semantics_hold(&self, section_id: Uuid, anchor: &RegionAnchor) -> bool {
        let Some(fallback) = anchor.text_fallback.as_ref() else {
            return true;
        };
        if sha256_text(&fallback.exact) != anchor.content_sha256 {
            return true;
        }
        self.fallback_match_count(section_id, &anchor.locator, fallback) > 0
    }

    fn fallback_match_count(
        &self,
        section_id: Uuid,
        locator: &RegionLocator,
        fallback: &TextQuote,
    ) -> usize {
        let mut count = 0usize;
        for block in self.blocks.iter().filter(|block| {
            if block.section_id != section_id {
                return false;
            }
            match (locator, block.locator.as_ref()) {
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
                (RegionLocator::Docx { block_id }, _) => block_id == &block.id,
                _ => false,
            }
        }) {
            count = count.saturating_add(block.plain_text.match_indices(&fallback.exact).count());
            if count > 1 {
                break;
            }
        }
        count
    }

    fn block_info(&self, section_id: Uuid, block_id: Uuid) -> Option<(i64, u32)> {
        let block = self
            .blocks
            .iter()
            .find(|block| block.section_id == section_id && block.id == block_id)?;
        let length = u32::try_from(block.plain_text.chars().count()).ok()?;
        Some((block.ordinal, length))
    }
}

fn valid_normalized_rect(rect: &crate::domain::NormalizedRect) -> bool {
    [rect.x, rect.y, rect.width, rect.height]
        .iter()
        .all(|value| value.is_finite() && (0.0..=1.0).contains(value))
        && rect.x + rect.width <= 1.0
        && rect.y + rect.height <= 1.0
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn insert_ai_conversation_annotation(
    transaction: &mut Transaction<'_, Sqlite>,
    annotation_id: Uuid,
    book_id: Uuid,
    section_id: Uuid,
    anchor_json: &str,
    selected_text: Option<&str>,
    conversation_id: Uuid,
    timestamp: &str,
) -> AppResult<()> {
    let result = sqlx::query(
        "INSERT INTO annotations (id, book_id, section_id, kind, anchor_json, selected_text, conversation_id, revision, created_at, updated_at) VALUES (?, ?, ?, 'ai_conversation', ?, ?, ?, 1, ?, ?)",
    )
    .bind(annotation_id.to_string())
    .bind(book_id.to_string())
    .bind(section_id.to_string())
    .bind(anchor_json)
    .bind(selected_text)
    .bind(conversation_id.to_string())
    .bind(timestamp)
    .bind(timestamp)
    .execute(&mut **transaction)
    .await?;
    if result.rows_affected() != 1 {
        return Err(AppError::new(AppErrorCode::DatabaseError));
    }
    Ok(())
}

pub(crate) async fn delete_ai_conversation_annotation(
    transaction: &mut Transaction<'_, Sqlite>,
    annotation_id: Uuid,
    book_id: Uuid,
    conversation_id: Uuid,
) -> AppResult<u64> {
    Ok(sqlx::query(
        "DELETE FROM annotations WHERE id = ? AND book_id = ? AND kind = 'ai_conversation' AND conversation_id = ?",
    )
    .bind(annotation_id.to_string())
    .bind(book_id.to_string())
    .bind(conversation_id.to_string())
    .execute(&mut **transaction)
    .await?
    .rows_affected())
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

#[cfg(test)]
mod summary_tests {
    use super::{default_summary, normalize_summary};

    #[test]
    fn default_summary_uses_the_first_short_ai_sentence() {
        assert_eq!(
            default_summary("  This is the short explanation. More detail follows.  "),
            "This is the short explanation."
        );
    }

    #[test]
    fn edited_summary_is_trimmed_and_bounded() {
        assert_eq!(
            normalize_summary("  My description  ").unwrap(),
            "My description"
        );
        assert!(normalize_summary("").is_err());
        assert!(normalize_summary(&"x".repeat(513)).is_err());
    }
}
