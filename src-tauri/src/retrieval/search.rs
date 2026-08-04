use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool, sqlite::SqliteRow};
use uuid::Uuid;

use crate::{
    domain::{ContentSource, DocumentLocator},
    errors::{AppError, AppErrorCode, AppResult},
};

use super::provenance::RetrievalProvenance;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    pub snippet: String,
    pub locator: DocumentLocator,
    pub section_title: Option<String>,
    pub provenance: RetrievalProvenance,
}

struct RankedHit {
    hit: SearchHit,
    score: f64,
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
            .total_cmp(&left.score)
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
        "SELECT c.id AS stable_id, c.text, c.locator_json, s.title AS section_title, bm25(search_chunks_fts) AS relevance FROM search_chunks_fts JOIN search_chunks c ON c.rowid = search_chunks_fts.rowid JOIN sections s ON s.id = c.section_id AND s.book_id = c.book_id WHERE search_chunks_fts MATCH ? AND c.book_id = ? ORDER BY relevance, s.ordinal, c.ordinal LIMIT ?",
    )
    .bind(&phrase)
    .bind(book_id.to_string())
    .bind(fetch_limit)
    .fetch_all(pool)
    .await?;
    let indexed_rows = sqlx::query(
        "SELECT c.id AS stable_id, c.text, c.locator_json, c.source, c.page_id, c.block_id, c.correction_id, correction.original_value_sha256, bm25(index_search_chunks_fts) AS relevance FROM index_search_chunks_fts JOIN index_search_chunks c ON c.rowid = index_search_chunks_fts.rowid JOIN index_pages p ON p.id = c.page_id AND p.book_id = c.book_id AND p.content_version = c.content_version LEFT JOIN index_corrections correction ON correction.id = c.correction_id AND correction.page_id = c.page_id AND correction.book_id = c.book_id WHERE index_search_chunks_fts MATCH ? AND c.book_id = ? ORDER BY relevance, p.page_number, c.ordinal LIMIT ?",
    )
    .bind(phrase)
    .bind(book_id.to_string())
    .bind(fetch_limit)
    .fetch_all(pool)
    .await?;

    let mut hits = Vec::with_capacity(local_rows.len() + indexed_rows.len());
    for row in local_rows {
        let provenance = RetrievalProvenance::local_text();
        let relevance = fts_score(row.try_get::<f64, _>("relevance")?);
        hits.push(RankedHit {
            score: relevance * provenance.ranking_weight(),
            source_order: source_order(provenance.source),
            stable_order: row.try_get("stable_id")?,
            hit: SearchHit {
                snippet: snippet(&row.try_get::<String, _>("text")?, query),
                locator: parse_locator(&row)?,
                section_title: row.try_get("section_title")?,
                provenance,
            },
        });
    }
    for row in indexed_rows {
        hits.push(indexed_ranked_hit(&row, query, true)?);
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
        "SELECT c.id AS stable_id, c.text, c.locator_json, s.title AS section_title FROM search_chunks c JOIN sections s ON s.id = c.section_id AND s.book_id = c.book_id WHERE c.book_id = ? AND c.text LIKE '%' || ? || '%' ESCAPE '\\' ORDER BY s.ordinal, c.ordinal LIMIT ?",
    )
    .bind(book_id.to_string())
    .bind(&escaped)
    .bind(fetch_limit)
    .fetch_all(pool)
    .await?;
    let indexed_rows = sqlx::query(
        "SELECT c.id AS stable_id, c.text, c.locator_json, c.source, c.page_id, c.block_id, c.correction_id, correction.original_value_sha256 FROM index_search_chunks c JOIN index_pages p ON p.id = c.page_id AND p.book_id = c.book_id AND p.content_version = c.content_version LEFT JOIN index_corrections correction ON correction.id = c.correction_id AND correction.page_id = c.page_id AND correction.book_id = c.book_id WHERE c.book_id = ? AND c.text LIKE '%' || ? || '%' ESCAPE '\\' ORDER BY p.page_number, c.ordinal LIMIT ?",
    )
    .bind(book_id.to_string())
    .bind(escaped)
    .bind(fetch_limit)
    .fetch_all(pool)
    .await?;

    let mut hits = Vec::with_capacity(local_rows.len() + indexed_rows.len());
    for row in local_rows {
        let text: String = row.try_get("text")?;
        let provenance = RetrievalProvenance::local_text();
        hits.push(RankedHit {
            score: like_score(&text, query) * provenance.ranking_weight(),
            source_order: source_order(provenance.source),
            stable_order: row.try_get("stable_id")?,
            hit: SearchHit {
                snippet: snippet(&text, query),
                locator: parse_locator(&row)?,
                section_title: row.try_get("section_title")?,
                provenance,
            },
        });
    }
    for row in indexed_rows {
        hits.push(indexed_ranked_hit(&row, query, false)?);
    }
    Ok(hits)
}

fn indexed_ranked_hit(row: &SqliteRow, query: &str, fts: bool) -> AppResult<RankedHit> {
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
    let provenance = RetrievalProvenance::indexed(
        source,
        page_id,
        block_id,
        correction_id,
        original_value_sha256,
    );
    let text: String = row.try_get("text")?;
    let raw_score = if fts {
        fts_score(row.try_get::<f64, _>("relevance")?)
    } else {
        like_score(&text, query)
    };
    Ok(RankedHit {
        score: raw_score * provenance.ranking_weight(),
        source_order: source_order(source),
        stable_order: row.try_get("stable_id")?,
        hit: SearchHit {
            snippet: snippet(&text, query),
            locator: parse_locator(row)?,
            section_title: None,
            provenance,
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

fn fts_score(rank: f64) -> f64 {
    if rank.is_finite() {
        (-rank).max(f64::MIN_POSITIVE)
    } else {
        f64::MIN_POSITIVE
    }
}

fn like_score(text: &str, query: &str) -> f64 {
    let occurrences = text.matches(query).count().max(1) as f64;
    occurrences / text.chars().count().max(1) as f64
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
