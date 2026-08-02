use std::collections::HashSet;

use sqlx::{Row, Sqlite, SqlitePool, Transaction};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{
    book_repository as books,
    domain::{
        BlockKind, BookFormat, BookSummary, DocumentLocator, ImportErrorStage,
        NormalizedBlockInput, NormalizedRect, NormalizedSectionInput, stable_block_id,
        stable_section_id,
    },
    errors::{AppError, AppErrorCode, AppResult},
    retrieval::chunker::{ChunkBlock, chunk_blocks_default},
};

const MAX_BATCH_SECTIONS: usize = 25;
const MAX_BATCH_BLOCKS: usize = 500;
const MAX_BATCH_JSON_BYTES: usize = 8 * 1024 * 1024;

pub async fn begin_parse(
    pool: &SqlitePool,
    book_id: Uuid,
    title: &str,
    author: Option<&str>,
    language: Option<&str>,
) -> AppResult<()> {
    if title.trim().is_empty() {
        return Err(AppError::new(AppErrorCode::InvalidInput));
    }
    let mut transaction = pool.begin().await?;
    let status: Option<String> = sqlx::query_scalar("SELECT import_status FROM books WHERE id = ?")
        .bind(book_id.to_string())
        .fetch_optional(&mut *transaction)
        .await?;
    match status.as_deref() {
        Some("parsing" | "ready") => {}
        Some(_) => return Err(AppError::new(AppErrorCode::RequestConflict)),
        None => return Err(AppError::new(AppErrorCode::NotFound)),
    }
    sqlx::query("DELETE FROM sections WHERE book_id = ?")
        .bind(book_id.to_string())
        .execute(&mut *transaction)
        .await?;
    let updated = sqlx::query(
        "UPDATE books SET title = ?, author = ?, language = ?, import_status = 'parsing', import_error_code = NULL, import_error_message = NULL, import_error_stage = NULL, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ? AND import_status IN ('parsing', 'ready')",
    )
    .bind(title.trim())
    .bind(author)
    .bind(language)
    .bind(book_id.to_string())
    .execute(&mut *transaction)
    .await?;
    ensure_changed(updated.rows_affected())?;
    transaction.commit().await?;
    Ok(())
}

pub async fn append_parsed_sections(
    pool: &SqlitePool,
    book_id: Uuid,
    sections: &[NormalizedSectionInput],
) -> AppResult<()> {
    let result = append_parsed_sections_transaction(pool, book_id, sections).await;
    if let Err(error) = &result
        && matches!(
            error.code,
            AppErrorCode::InvalidInput | AppErrorCode::DatabaseError
        )
    {
        let _ = mark_failed_and_clear(
            pool,
            book_id,
            ImportErrorStage::Parsing,
            error.code,
            error.code.user_message(),
            false,
            Some("parsing"),
        )
        .await;
    }
    result
}

async fn append_parsed_sections_transaction(
    pool: &SqlitePool,
    book_id: Uuid,
    sections: &[NormalizedSectionInput],
) -> AppResult<()> {
    let serialized =
        serde_json::to_vec(sections).map_err(|_| AppError::new(AppErrorCode::InvalidInput))?;
    if serialized.len() > MAX_BATCH_JSON_BYTES {
        return Err(AppError::new(AppErrorCode::InvalidInput));
    }
    let mut transaction = pool.begin().await?;
    let row = sqlx::query("SELECT format, import_status FROM books WHERE id = ?")
        .bind(book_id.to_string())
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or_else(|| AppError::new(AppErrorCode::NotFound))?;
    if row.try_get::<String, _>("import_status")? != "parsing" {
        return Err(AppError::new(AppErrorCode::BookNotReady));
    }
    let format = parse_format(&row.try_get::<String, _>("format")?)?;
    validate_batch(book_id, &format, sections)?;
    reject_existing_collisions(&mut transaction, book_id, sections).await?;

    let batch_ids = sections
        .iter()
        .map(|section| section.id)
        .collect::<HashSet<_>>();
    let mut inserted_ids = HashSet::new();
    let mut pending = sections.iter().collect::<Vec<_>>();
    while !pending.is_empty() {
        let before = pending.len();
        let mut deferred = Vec::new();
        for section in pending {
            let parent_ready = section.parent_id.is_none_or(|parent_id| {
                !batch_ids.contains(&parent_id) || inserted_ids.contains(&parent_id)
            });
            if !parent_ready {
                deferred.push(section);
                continue;
            }
            sqlx::query(
            "INSERT INTO sections (id, book_id, parent_id, ordinal, title, locator_json) VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(section.id.to_string())
        .bind(book_id.to_string())
        .bind(section.parent_id.map(|id| id.to_string()))
        .bind(i64::from(section.ordinal))
        .bind(section.title.trim())
        .bind(locator_json(&section.locator)?)
        .execute(&mut *transaction)
        .await?;
            inserted_ids.insert(section.id);
        }
        if deferred.len() == before {
            return Err(AppError::new(AppErrorCode::InvalidInput));
        }
        pending = deferred;
    }
    for section in sections {
        for block in &section.blocks {
            sqlx::query(
                "INSERT INTO blocks (id, book_id, section_id, ordinal, kind, plain_text, locator_json) VALUES (?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(block.id.to_string())
            .bind(book_id.to_string())
            .bind(section.id.to_string())
            .bind(i64::from(block.ordinal))
            .bind(block_kind_name(&block.kind))
            .bind(&block.plain_text)
            .bind(locator_json(&block.locator)?)
            .execute(&mut *transaction)
            .await?;
        }
    }
    transaction.commit().await?;
    Ok(())
}

pub async fn finalize_import(
    pool: &SqlitePool,
    book_id: Uuid,
    cancellation: &CancellationToken,
) -> AppResult<BookSummary> {
    let result = finalize_import_transaction(pool, book_id, cancellation).await;
    if let Err(error) = &result {
        let _ = mark_failed_and_clear(
            pool,
            book_id,
            ImportErrorStage::Indexing,
            error.code,
            error.code.user_message(),
            false,
            Some("indexing"),
        )
        .await;
    }
    result
}

async fn finalize_import_transaction(
    pool: &SqlitePool,
    book_id: Uuid,
    cancellation: &CancellationToken,
) -> AppResult<BookSummary> {
    ensure_not_cancelled(cancellation)?;
    let mut transaction = pool.begin().await?;
    let status: Option<String> = sqlx::query_scalar("SELECT import_status FROM books WHERE id = ?")
        .bind(book_id.to_string())
        .fetch_optional(&mut *transaction)
        .await?;
    match status.as_deref() {
        Some("indexing") => {}
        Some(_) => return Err(AppError::new(AppErrorCode::BookNotReady)),
        None => return Err(AppError::new(AppErrorCode::NotFound)),
    }

    let rows = sqlx::query(
        "SELECT s.id AS section_id, s.ordinal AS section_ordinal, b.ordinal AS block_ordinal, b.plain_text, b.locator_json FROM sections s JOIN blocks b ON b.section_id = s.id AND b.book_id = s.book_id WHERE s.book_id = ? ORDER BY s.ordinal, b.ordinal",
    )
    .bind(book_id.to_string())
    .fetch_all(&mut *transaction)
    .await?;
    if rows.is_empty() {
        return Err(AppError::new(AppErrorCode::NoExtractableText));
    }
    let mut section_ordinals = std::collections::HashMap::<Uuid, u32>::new();
    let mut blocks = Vec::with_capacity(rows.len());
    for row in rows {
        let section_id = parse_uuid(&row.try_get::<String, _>("section_id")?)?;
        let section_ordinal = u32::try_from(row.try_get::<i64, _>("section_ordinal")?)
            .map_err(|_| AppError::new(AppErrorCode::DatabaseError))?;
        let block_ordinal = u32::try_from(row.try_get::<i64, _>("block_ordinal")?)
            .map_err(|_| AppError::new(AppErrorCode::DatabaseError))?;
        section_ordinals.insert(section_id, section_ordinal);
        blocks.push(ChunkBlock {
            section_id,
            ordinal: block_ordinal,
            text: row.try_get("plain_text")?,
            locator_json: row.try_get("locator_json")?,
        });
    }
    let chunks = chunk_blocks_default(&blocks);
    if chunks.is_empty() {
        return Err(AppError::new(AppErrorCode::NoExtractableText));
    }

    sqlx::query("DELETE FROM search_chunks WHERE book_id = ?")
        .bind(book_id.to_string())
        .execute(&mut *transaction)
        .await?;
    for chunk in chunks {
        ensure_not_cancelled(cancellation)?;
        let section_ordinal = section_ordinals
            .get(&chunk.section_id)
            .copied()
            .ok_or_else(|| AppError::new(AppErrorCode::DatabaseError))?;
        let id = Uuid::new_v5(
            &book_id,
            format!("chunk:{section_ordinal}:{}", chunk.ordinal).as_bytes(),
        );
        sqlx::query(
            "INSERT INTO search_chunks (id, book_id, section_id, ordinal, text, locator_json, token_estimate) VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(id.to_string())
        .bind(book_id.to_string())
        .bind(chunk.section_id.to_string())
        .bind(i64::from(chunk.ordinal))
        .bind(chunk.text)
        .bind(chunk.locator_json)
        .bind(i64::try_from(chunk.code_point_count).unwrap_or(i64::MAX))
        .execute(&mut *transaction)
        .await?;
    }
    ensure_not_cancelled(cancellation)?;
    let updated = sqlx::query(
        "UPDATE books SET import_status = 'ready', import_error_code = NULL, import_error_message = NULL, import_error_stage = NULL, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ? AND import_status = 'indexing'",
    )
    .bind(book_id.to_string())
    .execute(&mut *transaction)
    .await?;
    ensure_changed(updated.rows_affected())?;
    ensure_not_cancelled(cancellation)?;
    transaction.commit().await?;
    Ok(books::get(pool, book_id).await?.summary)
}

#[allow(clippy::too_many_arguments)]
pub async fn mark_failed_and_clear(
    pool: &SqlitePool,
    book_id: Uuid,
    stage: ImportErrorStage,
    code: AppErrorCode,
    message: &str,
    clear_owned_source: bool,
    expected_status: Option<&str>,
) -> AppResult<()> {
    let mut transaction = pool.begin().await?;
    sqlx::query("DELETE FROM sections WHERE book_id = ?")
        .bind(book_id.to_string())
        .execute(&mut *transaction)
        .await?;
    let result = if clear_owned_source {
        sqlx::query(
            "UPDATE books SET import_status = 'failed', import_error_code = ?, import_error_message = ?, import_error_stage = ?, sha256 = NULL, stored_path = NULL, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ? AND (? IS NULL OR import_status = ?)",
        )
        .bind(error_code_name(code))
        .bind(message)
        .bind(error_stage_name(&stage))
        .bind(book_id.to_string())
        .bind(expected_status)
        .bind(expected_status)
        .execute(&mut *transaction)
        .await?
    } else {
        sqlx::query(
            "UPDATE books SET import_status = 'failed', import_error_code = ?, import_error_message = ?, import_error_stage = ?, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ? AND (? IS NULL OR import_status = ?)",
        )
        .bind(error_code_name(code))
        .bind(message)
        .bind(error_stage_name(&stage))
        .bind(book_id.to_string())
        .bind(expected_status)
        .bind(expected_status)
        .execute(&mut *transaction)
        .await?
    };
    if result.rows_affected() == 0 {
        transaction.rollback().await?;
        return Err(AppError::new(AppErrorCode::RequestConflict));
    }
    transaction.commit().await?;
    Ok(())
}

fn validate_batch(
    book_id: Uuid,
    format: &BookFormat,
    sections: &[NormalizedSectionInput],
) -> AppResult<()> {
    let block_count = sections
        .iter()
        .map(|section| section.blocks.len())
        .sum::<usize>();
    if sections.is_empty()
        || sections.len() > MAX_BATCH_SECTIONS
        || block_count == 0
        || block_count > MAX_BATCH_BLOCKS
    {
        return Err(AppError::new(AppErrorCode::InvalidInput));
    }
    let mut section_ids = HashSet::new();
    let mut section_ordinals = HashSet::new();
    for section in sections {
        if section.title.trim().is_empty()
            || section.id != stable_section_id(book_id, section.ordinal)
            || !section_ids.insert(section.id)
            || !section_ordinals.insert(section.ordinal)
            || section.parent_id == Some(section.id)
            || section.blocks.is_empty()
        {
            return Err(AppError::new(AppErrorCode::InvalidInput));
        }
        validate_locator(&section.locator, format, section, None)?;
        let mut block_ids = HashSet::new();
        let mut block_ordinals = HashSet::new();
        for block in &section.blocks {
            if block.plain_text.trim().is_empty()
                || block.id != stable_block_id(book_id, section.ordinal, block.ordinal)
                || !block_ids.insert(block.id)
                || !block_ordinals.insert(block.ordinal)
            {
                return Err(AppError::new(AppErrorCode::InvalidInput));
            }
            validate_locator(&block.locator, format, section, Some(block))?;
        }
    }
    Ok(())
}

async fn reject_existing_collisions(
    transaction: &mut Transaction<'_, Sqlite>,
    book_id: Uuid,
    sections: &[NormalizedSectionInput],
) -> AppResult<()> {
    let batch_ids = sections
        .iter()
        .map(|section| section.id)
        .collect::<HashSet<_>>();
    let parents = sections
        .iter()
        .filter_map(|section| section.parent_id.map(|parent| (section.id, parent)))
        .collect::<std::collections::HashMap<_, _>>();
    for section in sections {
        let mut seen = HashSet::new();
        let mut current = section.id;
        while let Some(parent) = parents.get(&current).copied() {
            if !seen.insert(current) || parent == section.id {
                return Err(AppError::new(AppErrorCode::InvalidInput));
            }
            current = parent;
        }
    }
    for section in sections {
        let collision: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sections WHERE id = ? OR (book_id = ? AND ordinal = ?)",
        )
        .bind(section.id.to_string())
        .bind(book_id.to_string())
        .bind(i64::from(section.ordinal))
        .fetch_one(&mut **transaction)
        .await?;
        if collision != 0 {
            return Err(AppError::new(AppErrorCode::InvalidInput));
        }
        if let Some(parent_id) = section.parent_id
            && !batch_ids.contains(&parent_id)
        {
            let valid_parent: i64 =
                sqlx::query_scalar("SELECT COUNT(*) FROM sections WHERE id = ? AND book_id = ?")
                    .bind(parent_id.to_string())
                    .bind(book_id.to_string())
                    .fetch_one(&mut **transaction)
                    .await?;
            if valid_parent != 1 {
                return Err(AppError::new(AppErrorCode::InvalidInput));
            }
        }
        for block in &section.blocks {
            let collision: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM blocks WHERE id = ?")
                .bind(block.id.to_string())
                .fetch_one(&mut **transaction)
                .await?;
            if collision != 0 {
                return Err(AppError::new(AppErrorCode::InvalidInput));
            }
        }
    }
    Ok(())
}

fn validate_locator(
    locator: &DocumentLocator,
    format: &BookFormat,
    section: &NormalizedSectionInput,
    block: Option<&NormalizedBlockInput>,
) -> AppResult<()> {
    let valid = match (locator, format) {
        (
            DocumentLocator::Pdf {
                start_page,
                end_page,
                rects_by_page,
            },
            BookFormat::Pdf,
        ) => {
            *start_page > 0
                && *end_page >= *start_page
                && rects_by_page.as_ref().is_none_or(|pages| {
                    pages.iter().all(|(page, rects)| {
                        (*start_page..=*end_page).contains(page)
                            && rects.iter().all(rect_is_normalized)
                    })
                })
        }
        (DocumentLocator::Epub { cfi, section_id }, BookFormat::Epub) => {
            !cfi.trim().is_empty() && *section_id == section.id
        }
        (
            DocumentLocator::Docx {
                start_block_id,
                start_offset,
                end_block_id,
                end_offset,
            },
            BookFormat::Docx,
        ) => {
            let start = section
                .blocks
                .iter()
                .find(|candidate| candidate.id == *start_block_id)
                .map(|candidate| (candidate.ordinal, candidate.plain_text.chars().count()));
            let end = section
                .blocks
                .iter()
                .find(|candidate| candidate.id == *end_block_id)
                .map(|candidate| (candidate.ordinal, candidate.plain_text.chars().count()));
            start.is_some_and(|(_, length)| *start_offset as usize <= length)
                && end.is_some_and(|(_, length)| *end_offset as usize <= length)
                && start
                    .zip(end)
                    .is_some_and(|((start_index, _), (end_index, _))| {
                        start_index < end_index
                            || (start_index == end_index && start_offset <= end_offset)
                    })
                && block.is_none_or(|expected| {
                    *start_block_id == expected.id && *end_block_id == expected.id
                })
        }
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        Err(AppError::new(AppErrorCode::InvalidInput))
    }
}

fn rect_is_normalized(rect: &NormalizedRect) -> bool {
    [rect.x, rect.y, rect.width, rect.height]
        .into_iter()
        .all(|value| value.is_finite() && (0.0..=1.0).contains(&value))
        && rect.x + rect.width <= 1.0
        && rect.y + rect.height <= 1.0
}

fn locator_json(locator: &DocumentLocator) -> AppResult<String> {
    serde_json::to_string(locator).map_err(|_| AppError::new(AppErrorCode::InvalidInput))
}

fn parse_uuid(value: &str) -> AppResult<Uuid> {
    Uuid::parse_str(value).map_err(|_| AppError::new(AppErrorCode::DatabaseError))
}

fn parse_format(value: &str) -> AppResult<BookFormat> {
    match value {
        "pdf" => Ok(BookFormat::Pdf),
        "epub" => Ok(BookFormat::Epub),
        "docx" => Ok(BookFormat::Docx),
        _ => Err(AppError::new(AppErrorCode::DatabaseError)),
    }
}

fn block_kind_name(kind: &BlockKind) -> &'static str {
    match kind {
        BlockKind::Heading => "heading",
        BlockKind::Paragraph => "paragraph",
        BlockKind::List => "list",
        BlockKind::Table => "table",
        BlockKind::Caption => "caption",
        BlockKind::Equation => "equation",
    }
}

fn error_stage_name(stage: &ImportErrorStage) -> &'static str {
    match stage {
        ImportErrorStage::Copying => "copying",
        ImportErrorStage::Parsing => "parsing",
        ImportErrorStage::Indexing => "indexing",
    }
}

fn error_code_name(code: AppErrorCode) -> &'static str {
    match code {
        AppErrorCode::InvalidApiKey => "INVALID_API_KEY",
        AppErrorCode::ModelNotFound => "MODEL_NOT_FOUND",
        AppErrorCode::ProviderPermissionDenied => "PROVIDER_PERMISSION_DENIED",
        AppErrorCode::ProviderRegionRestricted => "PROVIDER_REGION_RESTRICTED",
        AppErrorCode::RateLimited => "RATE_LIMITED",
        AppErrorCode::InsufficientQuota => "INSUFFICIENT_QUOTA",
        AppErrorCode::ContextTooLarge => "CONTEXT_TOO_LARGE",
        AppErrorCode::NetworkOffline => "NETWORK_OFFLINE",
        AppErrorCode::ProviderUnavailable => "PROVIDER_UNAVAILABLE",
        AppErrorCode::ProviderRefused => "PROVIDER_REFUSED",
        AppErrorCode::UnsupportedFileType => "UNSUPPORTED_FILE_TYPE",
        AppErrorCode::FileCorrupted => "FILE_CORRUPTED",
        AppErrorCode::FileEncryptedOrDrm => "FILE_ENCRYPTED_OR_DRM",
        AppErrorCode::NoExtractableText => "NO_EXTRACTABLE_TEXT",
        AppErrorCode::ImportCancelled => "IMPORT_CANCELLED",
        AppErrorCode::DatabaseError => "DATABASE_ERROR",
        AppErrorCode::CredentialStoreError => "CREDENTIAL_STORE_ERROR",
        AppErrorCode::AnchorNotFound => "ANCHOR_NOT_FOUND",
        AppErrorCode::InvalidInput => "INVALID_INPUT",
        AppErrorCode::NotFound => "NOT_FOUND",
        AppErrorCode::BookNotReady => "BOOK_NOT_READY",
        AppErrorCode::RequestConflict => "REQUEST_CONFLICT",
        AppErrorCode::LocalIoError => "LOCAL_IO_ERROR",
    }
}

fn ensure_changed(rows_affected: u64) -> AppResult<()> {
    if rows_affected == 1 {
        Ok(())
    } else {
        Err(AppError::new(AppErrorCode::RequestConflict))
    }
}

fn ensure_not_cancelled(cancellation: &CancellationToken) -> AppResult<()> {
    if cancellation.is_cancelled() {
        Err(AppError::new(AppErrorCode::ImportCancelled))
    } else {
        Ok(())
    }
}
