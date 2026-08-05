use std::fmt;

use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool, sqlite::SqliteRow};
use uuid::Uuid;

use crate::{
    domain::{CitationReviewStatus, ContentSource, DocumentLocator},
    errors::{AppError, AppErrorCode, AppResult},
};

use super::provenance::RetrievalProvenance;

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    pub snippet: String,
    pub locator: DocumentLocator,
    pub section_title: Option<String>,
    pub provenance: RetrievalProvenance,
    #[serde(skip)]
    pub(crate) context_book_id: Uuid,
    #[serde(skip)]
    pub(crate) context_section_id: Option<Uuid>,
    #[serde(skip)]
    pub(crate) context_stable_id: String,
    #[serde(skip)]
    pub(crate) context_ordinal: u32,
    #[serde(skip)]
    pub(crate) context_text: String,
    #[serde(skip)]
    pub(crate) relevance_micros: u32,
}

impl fmt::Debug for SearchHit {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SearchHit")
            .field(
                "snippet",
                &format_args!("<redacted:{} scalars>", self.snippet.chars().count()),
            )
            .field("locator", &"<redacted>")
            .field(
                "section_title_scalars",
                &self
                    .section_title
                    .as_ref()
                    .map(|title| title.chars().count()),
            )
            .field("provenance", &self.provenance)
            .finish()
    }
}

struct RankedHit {
    hit: SearchHit,
    score: u64,
    source_order: u8,
    stable_order: String,
}

pub async fn search_book(
    pool: &SqlitePool,
    book_id: Uuid,
    query: &str,
    limit: u32,
) -> AppResult<Vec<SearchHit>> {
    let query = query.trim();
    if query.is_empty() {
        return Err(AppError::new(AppErrorCode::InvalidInput));
    }
    let status: Option<String> = sqlx::query_scalar("SELECT import_status FROM books WHERE id = ?")
        .bind(book_id.to_string())
        .fetch_optional(pool)
        .await?;
    match status.as_deref() {
        Some("ready") => {}
        Some(_) => return Err(AppError::new(AppErrorCode::BookNotReady)),
        None => return Err(AppError::new(AppErrorCode::NotFound)),
    }

    let capped_limit = limit.clamp(1, 100);
    let fetch_limit = i64::from(capped_limit.saturating_mul(4));
    let mut hits = if query.chars().count() >= 3 {
        search_fts(pool, book_id, query, fetch_limit).await?
    } else {
        search_like(pool, book_id, query, fetch_limit).await?
    };
    hits.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| left.source_order.cmp(&right.source_order))
            .then_with(|| left.stable_order.cmp(&right.stable_order))
    });
    hits.truncate(usize::try_from(capped_limit).map_err(|_| database_error())?);
    Ok(hits.into_iter().map(|ranked| ranked.hit).collect())
}

async fn search_fts(
    pool: &SqlitePool,
    book_id: Uuid,
    query: &str,
    fetch_limit: i64,
) -> AppResult<Vec<RankedHit>> {
    let phrase = format!("\"{}\"", query.replace('"', "\"\""));
    let local_rows = sqlx::query(
        "SELECT c.id AS stable_id, c.section_id, c.ordinal, c.text, c.locator_json, s.title AS section_title FROM search_chunks_fts JOIN search_chunks c ON c.rowid = search_chunks_fts.rowid JOIN sections s ON s.id = c.section_id AND s.book_id = c.book_id WHERE search_chunks_fts MATCH ? AND c.book_id = ? ORDER BY s.ordinal, c.ordinal, c.id LIMIT ?",
    )
    .bind(&phrase)
    .bind(book_id.to_string())
    .bind(fetch_limit)
    .fetch_all(pool)
    .await?;
    let indexed_rows = sqlx::query(
        "SELECT c.id AS stable_id, c.ordinal, c.text, c.locator_json, c.source, c.page_id, c.block_id, c.correction_id, correction.original_value_sha256, p.status AS page_status, p.review_reason_code FROM index_search_chunks_fts JOIN index_search_chunks c ON c.rowid = index_search_chunks_fts.rowid JOIN index_pages p ON p.id = c.page_id AND p.book_id = c.book_id AND p.content_version = c.content_version AND p.status IN ('indexed', 'needs_review') LEFT JOIN index_corrections correction ON correction.id = c.correction_id AND correction.page_id = c.page_id AND correction.book_id = c.book_id WHERE index_search_chunks_fts MATCH ? AND c.book_id = ? AND (c.source != 'user_corrected' OR (correction.id IS NOT NULL AND correction.conflict_state = 'active' AND correction.target_content_version = c.content_version)) AND NOT (c.source = 'ai_transcribed' AND EXISTS (SELECT 1 FROM index_corrections active_correction WHERE active_correction.book_id = c.book_id AND active_correction.page_id = c.page_id AND active_correction.target_block_id = c.block_id AND active_correction.target_content_version = c.content_version AND active_correction.value_kind = 'text' AND active_correction.conflict_state = 'active')) AND NOT (c.source IN ('ai_transcribed', 'user_corrected') AND EXISTS (SELECT 1 FROM index_corrections conflicted_correction WHERE conflicted_correction.book_id = c.book_id AND conflicted_correction.page_id = c.page_id AND conflicted_correction.target_block_id = c.block_id AND conflicted_correction.conflict_state = 'conflict')) ORDER BY p.page_number, c.ordinal, c.source, c.id LIMIT ?",
    )
    .bind(phrase)
    .bind(book_id.to_string())
    .bind(fetch_limit)
    .fetch_all(pool)
    .await?;

    let mut hits = Vec::with_capacity(local_rows.len() + indexed_rows.len());
    for row in local_rows {
        hits.push(local_ranked_hit(&row, book_id, query)?);
    }
    for row in indexed_rows {
        hits.push(indexed_ranked_hit(&row, book_id, query)?);
    }
    Ok(hits)
}

async fn search_like(
    pool: &SqlitePool,
    book_id: Uuid,
    query: &str,
    fetch_limit: i64,
) -> AppResult<Vec<RankedHit>> {
    let escaped = escape_like(query);
    let local_rows = sqlx::query(
        "SELECT c.id AS stable_id, c.section_id, c.ordinal, c.text, c.locator_json, s.title AS section_title FROM search_chunks c JOIN sections s ON s.id = c.section_id AND s.book_id = c.book_id WHERE c.book_id = ? AND c.text LIKE '%' || ? || '%' ESCAPE '\\' ORDER BY s.ordinal, c.ordinal, c.id LIMIT ?",
    )
    .bind(book_id.to_string())
    .bind(&escaped)
    .bind(fetch_limit)
    .fetch_all(pool)
    .await?;
    let indexed_rows = sqlx::query(
        "SELECT c.id AS stable_id, c.ordinal, c.text, c.locator_json, c.source, c.page_id, c.block_id, c.correction_id, correction.original_value_sha256, p.status AS page_status, p.review_reason_code FROM index_search_chunks c JOIN index_pages p ON p.id = c.page_id AND p.book_id = c.book_id AND p.content_version = c.content_version AND p.status IN ('indexed', 'needs_review') LEFT JOIN index_corrections correction ON correction.id = c.correction_id AND correction.page_id = c.page_id AND correction.book_id = c.book_id WHERE c.book_id = ? AND c.text LIKE '%' || ? || '%' ESCAPE '\\' AND (c.source != 'user_corrected' OR (correction.id IS NOT NULL AND correction.conflict_state = 'active' AND correction.target_content_version = c.content_version)) AND NOT (c.source = 'ai_transcribed' AND EXISTS (SELECT 1 FROM index_corrections active_correction WHERE active_correction.book_id = c.book_id AND active_correction.page_id = c.page_id AND active_correction.target_block_id = c.block_id AND active_correction.target_content_version = c.content_version AND active_correction.value_kind = 'text' AND active_correction.conflict_state = 'active')) AND NOT (c.source IN ('ai_transcribed', 'user_corrected') AND EXISTS (SELECT 1 FROM index_corrections conflicted_correction WHERE conflicted_correction.book_id = c.book_id AND conflicted_correction.page_id = c.page_id AND conflicted_correction.target_block_id = c.block_id AND conflicted_correction.conflict_state = 'conflict')) ORDER BY p.page_number, c.ordinal, c.source, c.id LIMIT ?",
    )
    .bind(book_id.to_string())
    .bind(escaped)
    .bind(fetch_limit)
    .fetch_all(pool)
    .await?;

    let mut hits = Vec::with_capacity(local_rows.len() + indexed_rows.len());
    for row in local_rows {
        hits.push(local_ranked_hit(&row, book_id, query)?);
    }
    for row in indexed_rows {
        hits.push(indexed_ranked_hit(&row, book_id, query)?);
    }
    Ok(hits)
}

fn local_ranked_hit(row: &SqliteRow, book_id: Uuid, query: &str) -> AppResult<RankedHit> {
    let provenance = RetrievalProvenance::local_text();
    let text: String = row.try_get("text")?;
    let stable_id: String = row.try_get("stable_id")?;
    let section_id = parse_uuid(row.try_get::<String, _>("section_id")?)?;
    let ordinal = to_u32(row.try_get::<i64, _>("ordinal")?)?;
    let relevance_micros = weighted_relevance(&text, query, provenance.ranking_weight());
    Ok(RankedHit {
        score: u64::from(relevance_micros),
        source_order: source_order(provenance.source),
        stable_order: format!("local_text:{section_id}:{ordinal:010}:{stable_id}"),
        hit: SearchHit {
            snippet: snippet(&text, query),
            locator: parse_locator(row)?,
            section_title: row.try_get("section_title")?,
            provenance,
            context_book_id: book_id,
            context_section_id: Some(section_id),
            context_stable_id: stable_id,
            context_ordinal: ordinal,
            context_text: text,
            relevance_micros,
        },
    })
}

fn indexed_ranked_hit(row: &SqliteRow, book_id: Uuid, query: &str) -> AppResult<RankedHit> {
    let source = ContentSource::from_database(&row.try_get::<String, _>("source")?)
        .ok_or_else(database_error)?;
    if source == ContentSource::LocalText {
        return Err(database_error());
    }
    let page_id = parse_uuid(row.try_get::<String, _>("page_id")?)?;
    let block_id = parse_uuid(row.try_get::<String, _>("block_id")?)?;
    let correction_id = row
        .try_get::<Option<String>, _>("correction_id")?
        .map(parse_uuid)
        .transpose()?;
    let original_value_sha256: Option<String> = row.try_get("original_value_sha256")?;
    if (source == ContentSource::UserCorrected)
        != (correction_id.is_some() && original_value_sha256.is_some())
    {
        return Err(database_error());
    }
    let review_status = if source == ContentSource::UserCorrected {
        CitationReviewStatus::UserCorrected
    } else if row.try_get::<String, _>("page_status")? == "needs_review"
        || row
            .try_get::<Option<String>, _>("review_reason_code")?
            .is_some()
    {
        CitationReviewStatus::NeedsReview
    } else {
        CitationReviewStatus::Indexed
    };
    let provenance = RetrievalProvenance::indexed_with_review(
        source,
        page_id,
        block_id,
        correction_id,
        original_value_sha256,
        review_status,
    );
    let text: String = row.try_get("text")?;
    let stable_id: String = row.try_get("stable_id")?;
    let ordinal = to_u32(row.try_get::<i64, _>("ordinal")?)?;
    let relevance_micros = weighted_relevance(&text, query, provenance.ranking_weight());
    let stable_order = format!(
        "{}:{page_id}:{block_id}:{}:{ordinal:010}:{stable_id}",
        source.as_str(),
        correction_id.map_or_else(String::new, |id| id.to_string())
    );
    Ok(RankedHit {
        score: u64::from(relevance_micros),
        source_order: source_order(source),
        stable_order,
        hit: SearchHit {
            snippet: snippet(&text, query),
            locator: parse_locator(row)?,
            section_title: None,
            provenance,
            context_book_id: book_id,
            context_section_id: None,
            context_stable_id: stable_id,
            context_ordinal: ordinal,
            context_text: text,
            relevance_micros,
        },
    })
}

fn parse_locator(row: &SqliteRow) -> AppResult<DocumentLocator> {
    let locator_json: String = row.try_get("locator_json")?;
    serde_json::from_str(&locator_json).map_err(AppError::database)
}

fn parse_uuid(value: String) -> AppResult<Uuid> {
    Uuid::parse_str(&value).map_err(|_| database_error())
}

fn to_u32(value: i64) -> AppResult<u32> {
    u32::try_from(value).map_err(|_| database_error())
}

fn weighted_relevance(text: &str, query: &str, source_weight_micros: u32) -> u32 {
    let occurrences = u64::try_from(text.matches(query).count().max(1)).unwrap_or(u64::MAX);
    let text_scalars = u64::try_from(text.chars().count().max(1)).unwrap_or(u64::MAX);
    let query_scalars = u64::try_from(query.chars().count().max(1)).unwrap_or(u64::MAX);
    let occurrence_score = occurrences.saturating_mul(1_000_000);
    let density_score = occurrences
        .saturating_mul(query_scalars)
        .saturating_mul(1_000_000)
        .checked_div(text_scalars)
        .unwrap_or(0);
    let exact_bonus = u64::from(text.trim() == query).saturating_mul(2_000_000);
    let raw = occurrence_score
        .saturating_add(density_score)
        .saturating_add(exact_bonus);
    let weighted = raw
        .saturating_mul(u64::from(source_weight_micros))
        .checked_div(1_000_000)
        .unwrap_or(u64::MAX);
    u32::try_from(weighted).unwrap_or(u32::MAX)
}

const fn source_order(source: ContentSource) -> u8 {
    match source {
        ContentSource::UserCorrected => 0,
        ContentSource::LocalText => 1,
        ContentSource::AiTranscribed => 2,
        ContentSource::AiDescription => 3,
    }
}

fn escape_like(query: &str) -> String {
    query
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

fn snippet(text: &str, query: &str) -> String {
    const CONTEXT: usize = 80;
    const MAX_SNIPPET: usize = 200;

    let code_points = text.chars().collect::<Vec<_>>();
    let match_start = text
        .find(query)
        .map(|byte_index| text[..byte_index].chars().count())
        .unwrap_or(0);
    let query_length = query.chars().count();
    let start = match_start.saturating_sub(CONTEXT);
    let end = (match_start + query_length + CONTEXT)
        .max(start + MAX_SNIPPET.min(code_points.len().saturating_sub(start)))
        .min(code_points.len());
    let mut result = String::new();
    if start > 0 {
        result.push('…');
    }
    result.extend(&code_points[start..end]);
    if end < code_points.len() {
        result.push('…');
    }
    result
}

fn database_error() -> AppError {
    AppError::new(AppErrorCode::DatabaseError)
}

#[cfg(test)]
mod tests {
    use tokio_util::sync::CancellationToken;

    use crate::{
        db::{
            Database,
            indexing::{CreateIndexRun, create_index_run},
        },
        domain::{IndexPageBlockKind, IndexQualityReason, NormalizedRect},
        indexing::{
            commit::{PageCommitRequest, commit_validated_page},
            state,
            validator::{ValidatedBlock, ValidatedPage},
        },
    };

    use super::*;

    #[test]
    fn like_fallback_escapes_every_metacharacter() {
        assert_eq!(escape_like(r"100%_\done"), r"100\%\_\\done");
    }

    #[test]
    fn snippets_are_bounded_by_unicode_code_points() {
        let text = format!("{}能量{}", "前".repeat(150), "后".repeat(150));
        let result = snippet(&text, "能量");
        assert!(result.contains("能量"));
        assert!(result.chars().count() <= 202);
        assert!(result.starts_with('…') && result.ends_with('…'));
    }

    #[test]
    fn local_and_page_fts_are_unioned_with_source_ranking_and_strict_book_isolation() {
        let temporary = tempfile::tempdir().unwrap();
        let database = Database::open(temporary.path().join("retrieval.sqlite3")).unwrap();
        tauri::async_runtime::block_on(async {
            let profile_id = insert_profile(database.pool()).await;
            let target = insert_book(database.pool(), "Target retrieval book", "a").await;
            let decoy = insert_book(database.pool(), "More relevant decoy book", "b").await;
            insert_local_chunk(database.pool(), target, "A local spectral beacon passage.").await;
            insert_local_chunk(
                database.pool(),
                decoy,
                &format!("{} decoy-only marker", "spectral beacon ".repeat(80)),
            )
            .await;
            commit_ai_page(
                database.pool(),
                target,
                profile_id,
                "An AI spectral beacon transcription.",
                "A spectral beacon auxiliary visual description.",
            )
            .await;
            commit_ai_page(
                database.pool(),
                decoy,
                profile_id,
                &"spectral beacon ".repeat(60),
                "A decoy spectral beacon description.",
            )
            .await;

            let hits = search_book(database.pool(), target, "spectral beacon", 20)
                .await
                .unwrap();
            assert_eq!(
                hits.len(),
                3,
                "same wording from distinct sources is retained"
            );
            assert!(
                hits.iter()
                    .all(|hit| !hit.snippet.contains("decoy-only marker"))
            );
            assert!(hits.iter().any(|hit| {
                hit.provenance.source == ContentSource::LocalText
                    && hit.section_title.as_deref() == Some("Synthetic local section")
            }));
            assert!(hits.iter().any(|hit| {
                hit.provenance.source == ContentSource::AiTranscribed && hit.provenance.quoteable
            }));
            let description_index = hits
                .iter()
                .position(|hit| hit.provenance.source == ContentSource::AiDescription)
                .unwrap();
            let transcription_index = hits
                .iter()
                .position(|hit| hit.provenance.source == ContentSource::AiTranscribed)
                .unwrap();
            assert!(description_index > transcription_index);
            assert!(!hits[description_index].provenance.quoteable);
        });
    }

    async fn insert_profile(pool: &SqlitePool) -> Uuid {
        let profile_id = Uuid::new_v4();
        let timestamp = "2026-08-04T00:00:00.000Z";
        sqlx::query(
            "INSERT INTO provider_profiles (id, provider_kind, display_name, model_id, context_window_tokens, created_at, updated_at, validated_at) VALUES (?, 'openai', 'Synthetic retrieval profile', 'gpt-5.6', 1050000, ?, ?, ?)",
        )
        .bind(profile_id.to_string())
        .bind(timestamp)
        .bind(timestamp)
        .bind(timestamp)
        .execute(pool)
        .await
        .unwrap();
        profile_id
    }

    async fn insert_book(pool: &SqlitePool, title: &str, hash_character: &str) -> Uuid {
        let book_id = Uuid::new_v4();
        let timestamp = "2026-08-04T00:00:00.000Z";
        sqlx::query(
            "INSERT INTO books (id, sha256, title, format, original_filename, stored_path, import_status, created_at, updated_at) VALUES (?, ?, ?, 'pdf', 'synthetic.pdf', ?, 'ready', ?, ?)",
        )
        .bind(book_id.to_string())
        .bind(hash_character.repeat(64))
        .bind(title)
        .bind(format!("books/{book_id}/original.pdf"))
        .bind(timestamp)
        .bind(timestamp)
        .execute(pool)
        .await
        .unwrap();
        book_id
    }

    async fn insert_local_chunk(pool: &SqlitePool, book_id: Uuid, text: &str) {
        let section_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO sections (id, book_id, ordinal, title, locator_json) VALUES (?, ?, 0, 'Synthetic local section', ?)",
        )
        .bind(section_id.to_string())
        .bind(book_id.to_string())
        .bind(r#"{"format":"pdf","startPage":1,"endPage":1,"rectsByPage":null}"#.to_string())
        .execute(pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO search_chunks (id, book_id, section_id, ordinal, text, locator_json, token_estimate) VALUES (?, ?, ?, 0, ?, ?, 10)",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(book_id.to_string())
        .bind(section_id.to_string())
        .bind(text)
        .bind(r#"{"format":"pdf","startPage":1,"endPage":1,"rectsByPage":null}"#.to_string())
        .execute(pool)
        .await
        .unwrap();
    }

    async fn commit_ai_page(
        pool: &SqlitePool,
        book_id: Uuid,
        profile_id: Uuid,
        text: &str,
        description: &str,
    ) {
        let run_id = create_index_run(
            pool,
            CreateIndexRun {
                book_id,
                provider_profile_id: profile_id,
                analysis_schema_version: "textbooklens.page-analysis.v1".to_owned(),
                render_version: "synthetic-render-v1".to_owned(),
                parser_version: "synthetic-parser-v1".to_owned(),
            },
        )
        .await
        .unwrap();
        let page = state::queue(pool, run_id, 1, IndexQualityReason::NoText, None)
            .await
            .unwrap();
        let attempt_id = page.attempt_id.unwrap();
        state::claim_render(pool, page.page_id, attempt_id)
            .await
            .unwrap();
        state::mark_rendered(pool, page.page_id, attempt_id, &"c".repeat(64))
            .await
            .unwrap();
        state::claim_send(pool, page.page_id, attempt_id)
            .await
            .unwrap();
        state::mark_received(pool, page.page_id, attempt_id, &"d".repeat(64))
            .await
            .unwrap();
        let validated = ValidatedPage {
            page_number: 1,
            review_reason: None,
            blocks: vec![ValidatedBlock {
                ordinal: 0,
                kind: IndexPageBlockKind::Paragraph,
                plain_text: Some(text.to_owned()),
                latex: None,
                table_cells: None,
                visual_description: Some(description.to_owned()),
                bounds: Some(NormalizedRect::new(0.1, 0.1, 0.8, 0.2).unwrap()),
                source: ContentSource::AiTranscribed,
            }],
        };
        commit_validated_page(
            pool,
            PageCommitRequest {
                page_id: page.page_id,
                attempt_id,
                page: &validated,
            },
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    }
}
