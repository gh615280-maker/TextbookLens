use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::{
    domain::DocumentLocator,
    errors::{AppError, AppErrorCode, AppResult},
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    pub snippet: String,
    pub locator: DocumentLocator,
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

    let capped_limit = i64::from(limit.clamp(1, 100));
    let rows = if query.chars().count() >= 3 {
        let phrase = format!("\"{}\"", query.replace('"', "\"\""));
        sqlx::query(
            "SELECT c.text, c.locator_json FROM search_chunks_fts JOIN search_chunks c ON c.rowid = search_chunks_fts.rowid JOIN sections s ON s.id = c.section_id AND s.book_id = c.book_id WHERE search_chunks_fts MATCH ? AND c.book_id = ? ORDER BY bm25(search_chunks_fts), s.ordinal, c.ordinal LIMIT ?",
        )
        .bind(phrase)
        .bind(book_id.to_string())
        .bind(capped_limit)
        .fetch_all(pool)
        .await?
    } else {
        let escaped = escape_like(query);
        sqlx::query(
            "SELECT c.text, c.locator_json FROM search_chunks c JOIN sections s ON s.id = c.section_id AND s.book_id = c.book_id WHERE c.book_id = ? AND c.text LIKE '%' || ? || '%' ESCAPE '\\' ORDER BY s.ordinal, c.ordinal LIMIT ?",
        )
        .bind(book_id.to_string())
        .bind(escaped)
        .bind(capped_limit)
        .fetch_all(pool)
        .await?
    };

    rows.into_iter()
        .map(|row| {
            let text: String = row.try_get("text")?;
            let locator_json: String = row.try_get("locator_json")?;
            let locator = serde_json::from_str(&locator_json).map_err(AppError::database)?;
            Ok(SearchHit {
                snippet: snippet(&text, query),
                locator,
            })
        })
        .collect()
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

#[cfg(test)]
mod tests {
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
}
