use std::{collections::HashMap, sync::Arc, time::Duration};

use chrono::Utc;
use parking_lot::Mutex;
use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::{Row, SqlitePool};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{
    ai::registry::ProviderCapabilityRegistry,
    app_state::AppPaths,
    credentials::CredentialStore,
    db::{
        indexing::database_timestamp,
        providers::{credential_key, kimi_api_region_name, parse_kimi_api_region},
    },
    documents::source::read_book_source_bytes,
    domain::{KimiApiRegion, RemoteCleanupHandle},
    errors::{AppError, AppErrorCode, AppResult},
    indexing::remote_cleanup::{
        RuntimeRemoteResourceCleaner, finish_failed_tracking_compensation,
        load_extraction_remote_handle, store_extraction_remote_handle, sweep_remote_resource_ids,
    },
};

use super::kimi_files::{FileState, KimiFilesClient};

pub const EXTRACTION_VERSION: &str = "kimi-file-extract-v1";
const POLL_INTERVAL: Duration = Duration::from_millis(750);
const MAX_POLLS: usize = 240;
const CHUNK_CHARS: usize = 1_800;
const CHUNK_OVERLAP: usize = 180;

#[derive(Default)]
pub struct ExtractionCancellationRegistry {
    tokens: Mutex<HashMap<Uuid, CancellationToken>>,
}
impl ExtractionCancellationRegistry {
    pub fn begin(&self, book_id: Uuid) -> AppResult<CancellationToken> {
        let mut tokens = self.tokens.lock();
        if tokens.contains_key(&book_id) {
            return Err(AppError::new(AppErrorCode::RequestConflict));
        }
        let token = CancellationToken::new();
        tokens.insert(book_id, token.clone());
        Ok(token)
    }
    pub fn cancel(&self, book_id: Uuid) -> bool {
        self.tokens.lock().get(&book_id).is_some_and(|t| {
            t.cancel();
            true
        })
    }
    pub fn finish(&self, book_id: Uuid) {
        self.tokens.lock().remove(&book_id);
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractionSummary {
    pub id: Uuid,
    pub book_id: Uuid,
    pub status: String,
    pub content_chars: u64,
    pub chunk_count: u64,
    pub reused: bool,
}

pub struct PrepareDependencies<'a> {
    pub pool: &'a SqlitePool,
    pub paths: &'a AppPaths,
    pub credentials: Arc<dyn CredentialStore>,
    pub capabilities: ProviderCapabilityRegistry,
    pub client: &'a KimiFilesClient,
}

pub async fn prepare(
    dependencies: PrepareDependencies<'_>,
    book_id: Uuid,
    profile_id: Uuid,
    cancel: CancellationToken,
) -> AppResult<ExtractionSummary> {
    let PrepareDependencies {
        pool,
        paths,
        credentials,
        capabilities,
        client,
    } = dependencies;
    let binding = load_binding(pool, book_id, profile_id).await?;
    if client.region() != binding.kimi_api_region {
        return Err(AppError::new(AppErrorCode::InvalidInput));
    }
    let existing = sqlx::query("SELECT id, status, credential_fingerprint FROM book_extractions WHERE source_sha256 = ? AND provider_kind = 'kimi' AND extraction_version = ?")
        .bind(&binding.source_sha256).bind(EXTRACTION_VERSION).fetch_optional(pool).await?;
    let extraction_id = if let Some(row) = existing {
        let id = parse_uuid(row.try_get::<String, _>("id")?)?;
        let status: String = row.try_get("status")?;
        if status == "ready" {
            return summary(pool, id, true).await;
        }
        if row.try_get::<String, _>("credential_fingerprint")? != binding.fingerprint {
            cleanup_existing(pool, credentials.clone(), capabilities.clone(), id).await;
            let remaining: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM provider_remote_resources WHERE extraction_id = ? AND cleanup_status IN ('pending','failed','cleaning')")
                .bind(id.to_string()).fetch_one(pool).await?;
            if remaining != 0 {
                return Err(AppError::new(AppErrorCode::RequestConflict));
            }
            reset_binding(pool, id, &binding).await?;
        }
        id
    } else {
        create_extraction(pool, book_id, profile_id, &binding).await?
    };

    let credential = credentials.get(&credential_key(profile_id)).await?;
    let mut handle =
        load_extraction_remote_handle(pool, credentials.as_ref(), extraction_id).await?;
    if cancel.is_cancelled() {
        return cancel_extraction(
            pool,
            extraction_id,
            handle.as_ref(),
            credentials,
            capabilities,
        )
        .await;
    }

    if handle.is_none() {
        set_status(pool, extraction_id, "uploading", None, false).await?;
        let bytes = read_book_source_bytes(pool, paths, book_id).await?;
        let uploaded = match client.upload(&credential, bytes, cancel.clone()).await {
            Ok(uploaded) => uploaded,
            Err(error) => {
                if cancellation_error(&error, &cancel) {
                    return cancel_extraction(pool, extraction_id, None, credentials, capabilities)
                        .await;
                }
                mark_failure(pool, extraction_id, &error).await?;
                return Err(error);
            }
        };
        let remote = RemoteCleanupHandle::new_kimi(uploaded.id().clone(), binding.kimi_api_region);
        match store_extraction_remote_handle(pool, credentials.as_ref(), extraction_id, &remote)
            .await
        {
            Ok(_) => handle = Some(remote),
            Err(failure) => {
                let deleted = client
                    .delete(&credential, uploaded.id(), CancellationToken::new())
                    .await
                    .is_ok();
                finish_failed_tracking_compensation(credentials.as_ref(), failure, deleted).await;
                let error = AppError::new(AppErrorCode::CredentialStoreError);
                mark_failure(pool, extraction_id, &error).await?;
                return Err(error);
            }
        }
    }
    let handle = handle.ok_or_else(|| AppError::new(AppErrorCode::DatabaseError))?;
    set_status(pool, extraction_id, "processing", None, false).await?;
    for _ in 0..MAX_POLLS {
        if cancel.is_cancelled() {
            return cancel_extraction(
                pool,
                extraction_id,
                Some(&handle),
                credentials,
                capabilities,
            )
            .await;
        }
        match client
            .status(&credential, handle.opaque_id(), cancel.clone())
            .await
        {
            Ok(FileState::Processing) => tokio::time::sleep(POLL_INTERVAL).await,
            Ok(FileState::Failed) => {
                let error = AppError::provider_failure(
                    AppErrorCode::ProviderUnavailable,
                    "Kimi file extraction failed",
                );
                mark_failure(pool, extraction_id, &error).await?;
                cleanup_existing(pool, credentials, capabilities, extraction_id).await;
                return Err(error);
            }
            Ok(FileState::Ready) => {
                set_status(pool, extraction_id, "downloading", None, false).await?;
                let content = match client
                    .content(&credential, handle.opaque_id(), cancel.clone())
                    .await
                {
                    Ok(content) => content,
                    Err(error) => {
                        if cancellation_error(&error, &cancel) {
                            return cancel_extraction(
                                pool,
                                extraction_id,
                                Some(&handle),
                                credentials,
                                capabilities,
                            )
                            .await;
                        }
                        mark_failure(pool, extraction_id, &error).await?;
                        cleanup_existing(pool, credentials, capabilities, extraction_id).await;
                        return Err(error);
                    }
                };
                if let Err(error) = persist_content(pool, extraction_id, book_id, &content).await {
                    mark_failure(pool, extraction_id, &error).await?;
                    cleanup_existing(pool, credentials, capabilities, extraction_id).await;
                    return Err(error);
                }
                cleanup_existing(pool, credentials, capabilities, extraction_id).await;
                return summary(pool, extraction_id, false).await;
            }
            Err(error) => {
                if cancellation_error(&error, &cancel) {
                    return cancel_extraction(
                        pool,
                        extraction_id,
                        Some(&handle),
                        credentials,
                        capabilities,
                    )
                    .await;
                }
                mark_failure(pool, extraction_id, &error).await?;
                return Err(error);
            }
        }
    }
    let error = AppError::provider_failure(
        AppErrorCode::ProviderUnavailable,
        "Kimi file extraction timed out",
    );
    mark_failure(pool, extraction_id, &error).await?;
    Err(error)
}

struct Binding {
    source_sha256: String,
    model_id: String,
    fingerprint: String,
    kimi_api_region: KimiApiRegion,
}

async fn load_binding(pool: &SqlitePool, book_id: Uuid, profile_id: Uuid) -> AppResult<Binding> {
    let row = sqlx::query("SELECT b.sha256, b.format, b.import_status, p.provider_kind, p.model_id, p.validated_at, p.kimi_api_region FROM books b JOIN provider_profiles p ON p.id = ? WHERE b.id = ?")
        .bind(profile_id.to_string()).bind(book_id.to_string()).fetch_optional(pool).await?
        .ok_or_else(|| AppError::new(AppErrorCode::NotFound))?;
    if row.try_get::<String, _>("format")? != "pdf"
        || row.try_get::<String, _>("import_status")? != "ready"
    {
        return Err(AppError::new(AppErrorCode::BookNotReady));
    }
    if row.try_get::<String, _>("provider_kind")? != "kimi" {
        return Err(AppError::unsupported_provider_capability());
    }
    let source_sha256 = row
        .try_get::<Option<String>, _>("sha256")?
        .ok_or_else(|| AppError::new(AppErrorCode::BookNotReady))?;
    let model_id: String = row.try_get("model_id")?;
    let validated_at: String = row.try_get("validated_at")?;
    let kimi_api_region = parse_kimi_api_region(
        &row.try_get::<Option<String>, _>("kimi_api_region")?
            .ok_or_else(|| AppError::new(AppErrorCode::DatabaseError))?,
    )?;
    let fingerprint = hex(Sha256::digest(
        format!(
            "{profile_id}\0{model_id}\0{validated_at}\0{}",
            kimi_api_region_name(kimi_api_region)
        )
        .as_bytes(),
    ));
    Ok(Binding {
        source_sha256,
        model_id,
        fingerprint,
        kimi_api_region,
    })
}

async fn create_extraction(
    pool: &SqlitePool,
    book_id: Uuid,
    profile_id: Uuid,
    b: &Binding,
) -> AppResult<Uuid> {
    let id = Uuid::new_v4();
    let now = database_timestamp(Utc::now());
    sqlx::query("INSERT INTO book_extractions (id, book_id, source_sha256, provider_profile_id, provider_kind, model_id, extraction_version, credential_fingerprint, status, created_at, updated_at, kimi_api_region) VALUES (?, ?, ?, ?, 'kimi', ?, ?, ?, 'queued', ?, ?, ?)")
        .bind(id.to_string()).bind(book_id.to_string()).bind(&b.source_sha256).bind(profile_id.to_string())
        .bind(&b.model_id).bind(EXTRACTION_VERSION).bind(&b.fingerprint).bind(&now).bind(&now)
        .bind(kimi_api_region_name(b.kimi_api_region)).execute(pool).await?;
    Ok(id)
}

async fn reset_binding(pool: &SqlitePool, id: Uuid, b: &Binding) -> AppResult<()> {
    sqlx::query("UPDATE book_extractions SET model_id = ?, credential_fingerprint = ?, kimi_api_region = ?, status = 'queued', safe_error_code = NULL, retryable = 0, cancel_requested_at = NULL, completed_at = NULL, updated_at = ? WHERE id = ? AND status != 'ready'")
        .bind(&b.model_id).bind(&b.fingerprint).bind(kimi_api_region_name(b.kimi_api_region))
        .bind(database_timestamp(Utc::now())).bind(id.to_string()).execute(pool).await?;
    Ok(())
}

async fn set_status(
    pool: &SqlitePool,
    id: Uuid,
    status: &str,
    error: Option<&str>,
    retryable: bool,
) -> AppResult<()> {
    sqlx::query("UPDATE book_extractions SET status = ?, attempt_count = attempt_count + CASE WHEN ? = 'uploading' THEN 1 ELSE 0 END, safe_error_code = ?, retryable = ?, updated_at = ? WHERE id = ? AND status != 'ready'")
        .bind(status).bind(status).bind(error).bind(i64::from(retryable)).bind(database_timestamp(Utc::now())).bind(id.to_string()).execute(pool).await?;
    Ok(())
}

async fn mark_failure(pool: &SqlitePool, id: Uuid, error: &AppError) -> AppResult<()> {
    set_status(
        pool,
        id,
        "failed",
        Some(error.code.stable_code()),
        matches!(
            error.code,
            AppErrorCode::RateLimited
                | AppErrorCode::ProviderUnavailable
                | AppErrorCode::NetworkOffline
        ),
    )
    .await
}

fn cancellation_error(error: &AppError, cancel: &CancellationToken) -> bool {
    error.code == AppErrorCode::ImportCancelled || cancel.is_cancelled()
}

async fn persist_content(pool: &SqlitePool, id: Uuid, book_id: Uuid, raw: &str) -> AppResult<()> {
    let normalized = normalize(raw)?;
    let chunks = chunk(&normalized);
    if chunks.is_empty() {
        return Err(AppError::new(AppErrorCode::ProviderUnavailable));
    }
    let mut tx = pool.begin().await?;
    sqlx::query("UPDATE book_extractions SET status = 'indexing', normalized_text = ?, content_sha256 = ?, content_chars = ?, updated_at = ? WHERE id = ?")
        .bind(&normalized).bind(hex(Sha256::digest(normalized.as_bytes()))).bind(normalized.chars().count() as i64).bind(database_timestamp(Utc::now())).bind(id.to_string()).execute(&mut *tx).await?;
    sqlx::query("DELETE FROM extraction_chunks WHERE extraction_id = ?")
        .bind(id.to_string())
        .execute(&mut *tx)
        .await?;
    let now = database_timestamp(Utc::now());
    for c in chunks {
        sqlx::query("INSERT INTO extraction_chunks (id, extraction_id, book_id, ordinal, char_start, char_end, paragraph_start, paragraph_end, text, text_fingerprint, token_estimate, locator_json, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
            .bind(Uuid::new_v4().to_string()).bind(id.to_string()).bind(book_id.to_string()).bind(c.ordinal as i64)
            .bind(c.start as i64).bind(c.end as i64).bind(c.paragraph_start as i64).bind(c.paragraph_end as i64)
            .bind(&c.text).bind(hex(Sha256::digest(c.text.as_bytes()))).bind(c.text.chars().count().div_ceil(4).max(1) as i64)
            .bind(serde_json::json!({"format":"extracted_text","charStart":c.start,"charEnd":c.end,"paragraphStart":c.paragraph_start,"paragraphEnd":c.paragraph_end,"textFingerprint":hex(Sha256::digest(c.text.as_bytes()))}).to_string())
            .bind(&now).execute(&mut *tx).await?;
    }
    sqlx::query("UPDATE book_extractions SET status = 'ready', completed_at = ?, updated_at = ?, safe_error_code = NULL, retryable = 0 WHERE id = ?")
        .bind(&now).bind(&now).bind(id.to_string()).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(())
}

struct Chunk {
    ordinal: usize,
    start: usize,
    end: usize,
    paragraph_start: usize,
    paragraph_end: usize,
    text: String,
}
fn chunk(text: &str) -> Vec<Chunk> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut start = 0;
    while start < chars.len() {
        let target = (start + CHUNK_CHARS).min(chars.len());
        let end = if target < chars.len() {
            (start + CHUNK_CHARS / 2..=target)
                .rev()
                .find(|&i| chars[i - 1] == '\n')
                .unwrap_or(target)
        } else {
            target
        };
        let value: String = chars[start..end].iter().collect();
        let paragraph_start = chars[..start].iter().filter(|&&c| c == '\n').count();
        let paragraph_end = paragraph_start + value.chars().filter(|&c| c == '\n').count();
        if !value.trim().is_empty() {
            out.push(Chunk {
                ordinal: out.len(),
                start,
                end,
                paragraph_start,
                paragraph_end,
                text: value,
            });
        }
        if end == chars.len() {
            break;
        }
        start = end.saturating_sub(CHUNK_OVERLAP).max(start + 1);
    }
    out
}

fn normalize(raw: &str) -> AppResult<String> {
    if raw.contains('\0') {
        return Err(AppError::new(AppErrorCode::ProviderUnavailable));
    }
    let value = raw.replace("\r\n", "\n").replace('\r', "\n");
    let mut out = String::with_capacity(value.len());
    let mut blanks = 0;
    for line in value.lines() {
        let line = line.trim_end();
        if line.is_empty() {
            blanks += 1;
            if blanks > 1 {
                continue;
            }
        } else {
            blanks = 0;
        }
        out.push_str(line);
        out.push('\n');
    }
    Ok(out.trim().to_owned())
}

async fn cleanup_existing(
    pool: &SqlitePool,
    credentials: Arc<dyn CredentialStore>,
    capabilities: ProviderCapabilityRegistry,
    id: Uuid,
) {
    let ids: Vec<String> = sqlx::query_scalar("SELECT id FROM provider_remote_resources WHERE extraction_id = ? AND cleanup_status IN ('pending','failed')").bind(id.to_string()).fetch_all(pool).await.unwrap_or_default();
    let ids = ids
        .into_iter()
        .filter_map(|v| Uuid::parse_str(&v).ok())
        .collect::<Vec<_>>();
    if ids.is_empty() {
        return;
    }
    let cleaner = RuntimeRemoteResourceCleaner::new(credentials.clone(), capabilities);
    let _ = sweep_remote_resource_ids(
        pool,
        credentials.as_ref(),
        &cleaner,
        &ids,
        CancellationToken::new(),
    )
    .await;
}

async fn cancel_extraction(
    pool: &SqlitePool,
    id: Uuid,
    _handle: Option<&RemoteCleanupHandle>,
    credentials: Arc<dyn CredentialStore>,
    capabilities: ProviderCapabilityRegistry,
) -> AppResult<ExtractionSummary> {
    let now = database_timestamp(Utc::now());
    sqlx::query("UPDATE book_extractions SET status = 'cancelled', cancel_requested_at = ?, completed_at = ?, updated_at = ? WHERE id = ? AND status != 'ready'").bind(&now).bind(&now).bind(&now).bind(id.to_string()).execute(pool).await?;
    cleanup_existing(pool, credentials, capabilities, id).await;
    summary(pool, id, false).await
}

pub async fn summary(pool: &SqlitePool, id: Uuid, reused: bool) -> AppResult<ExtractionSummary> {
    let row = sqlx::query("SELECT book_id, status, content_chars, (SELECT COUNT(*) FROM extraction_chunks c WHERE c.extraction_id = book_extractions.id) AS chunks FROM book_extractions WHERE id = ?")
        .bind(id.to_string()).fetch_optional(pool).await?.ok_or_else(|| AppError::new(AppErrorCode::NotFound))?;
    Ok(ExtractionSummary {
        id,
        book_id: parse_uuid(row.try_get("book_id")?)?,
        status: row.try_get("status")?,
        content_chars: row.try_get::<i64, _>("content_chars")? as u64,
        chunk_count: row.try_get::<i64, _>("chunks")? as u64,
        reused,
    })
}

/// Repairs only durable local state. It deliberately performs no network call;
/// the next explicit user start resumes polling/downloading using the vaulted id.
pub async fn recover_on_startup(pool: &SqlitePool) -> AppResult<()> {
    let now = database_timestamp(Utc::now());
    sqlx::query("UPDATE book_extractions SET status = CASE WHEN status = 'cancelling' THEN 'cancelled' WHEN EXISTS (SELECT 1 FROM provider_remote_resources r WHERE r.extraction_id = book_extractions.id AND r.cleanup_status IN ('pending','failed','cleaning')) THEN 'processing' ELSE 'queued' END, cancel_requested_at = CASE WHEN status = 'cancelling' THEN COALESCE(cancel_requested_at, ?) ELSE cancel_requested_at END, completed_at = CASE WHEN status = 'cancelling' THEN ? ELSE completed_at END, updated_at = ? WHERE status IN ('uploading','processing','downloading','indexing','cancelling')")
        .bind(&now).bind(&now).bind(&now).execute(pool).await?;
    Ok(())
}
fn parse_uuid(value: String) -> AppResult<Uuid> {
    Uuid::parse_str(&value).map_err(|_| AppError::new(AppErrorCode::DatabaseError))
}
fn hex(bytes: impl AsRef<[u8]>) -> String {
    bytes.as_ref().iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ai::registry::ProviderCapabilityRegistry,
        app_state::AppPaths,
        credentials::{CredentialStore, MemoryCredentialStore},
        db::Database,
        domain::DocumentLocator,
        retrieval::{context::book_candidate_from_search_hit, search::search_book},
    };
    use secrecy::SecretString;
    use std::{fs, sync::Arc};
    use tokio::sync::Notify;
    use wiremock::{
        Mock, MockServer, Request, ResponseTemplate,
        matchers::{method, path},
    };

    #[test]
    fn normalization_and_chunking_are_local_deterministic_and_offset_based() {
        let normalized = normalize("A\r\n\r\n\r\nB  \r\n").unwrap();
        assert_eq!(normalized, "A\n\nB");
        let chunks = chunk(&format!(
            "alpha spectral beacon\n{}omega",
            "x".repeat(2_200)
        ));
        assert!(chunks.len() >= 2);
        assert_eq!(chunks[0].start, 0);
        assert!(
            chunks
                .windows(2)
                .all(|pair| pair[1].start < pair[0].end && pair[1].end > pair[0].end)
        );
    }

    #[test]
    fn persisted_extraction_is_unique_and_fts_returns_only_relevant_local_chunks() {
        let temporary = tempfile::tempdir().unwrap();
        let database = Database::open(temporary.path().join("extraction.sqlite3")).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let book_id = Uuid::new_v4(); let profile_id = Uuid::new_v4(); let extraction_id = Uuid::new_v4();
            let now = database_timestamp(Utc::now()); let hash = "a".repeat(64);
            sqlx::query("INSERT INTO books (id, sha256, title, format, original_filename, stored_path, import_status, created_at, updated_at) VALUES (?, ?, 'Fixture', 'pdf', 'fixture.pdf', 'source.pdf', 'ready', ?, ?)")
                .bind(book_id.to_string()).bind(&hash).bind(&now).bind(&now).execute(database.pool()).await.unwrap();
            sqlx::query("INSERT INTO provider_profiles (id, provider_kind, display_name, model_id, context_window_tokens, created_at, updated_at, validated_at) VALUES (?, 'kimi', 'Fixture Kimi', 'kimi-k3', 1000000, ?, ?, ?)")
                .bind(profile_id.to_string()).bind(&now).bind(&now).bind(&now).execute(database.pool()).await.unwrap();
            sqlx::query("INSERT INTO book_extractions (id, book_id, source_sha256, provider_profile_id, provider_kind, model_id, extraction_version, credential_fingerprint, status, created_at, updated_at) VALUES (?, ?, ?, ?, 'kimi', 'kimi-k3', ?, ?, 'indexing', ?, ?)")
                .bind(extraction_id.to_string()).bind(book_id.to_string()).bind(&hash).bind(profile_id.to_string()).bind(EXTRACTION_VERSION).bind("f".repeat(64)).bind(&now).bind(&now).execute(database.pool()).await.unwrap();
            let full = format!("spectral beacon explains the target concept.\n{}\nunrelated appendix", "context ".repeat(400));
            persist_content(database.pool(), extraction_id, book_id, &full).await.unwrap();
            let ready = summary(database.pool(), extraction_id, false).await.unwrap();
            assert_eq!(ready.status, "ready"); assert!(ready.chunk_count > 1);
            let mut hits = search_book(database.pool(), book_id, "spectral beacon", 3).await.unwrap();
            assert!(!hits.is_empty()); assert!(hits.len() <= 3);
            assert!(hits.iter().all(|hit| hit.snippet.len() < full.len()));
            assert!(matches!(hits[0].locator, DocumentLocator::ExtractedText { .. }));
            let candidate = book_candidate_from_search_hit(book_id, hits.remove(0)).unwrap();
            assert!(candidate.text.len() < full.len(), "Q&A preparation must not inject the whole book");
            assert!(matches!(candidate.locator, Some(DocumentLocator::ExtractedText { .. })));
            let duplicate = sqlx::query("INSERT INTO book_extractions (id, book_id, source_sha256, provider_kind, model_id, extraction_version, credential_fingerprint, status, created_at, updated_at) VALUES (?, ?, ?, 'kimi', 'other', ?, 'x', 'queued', ?, ?)")
                .bind(Uuid::new_v4().to_string()).bind(book_id.to_string()).bind(&hash).bind(EXTRACTION_VERSION).bind(&now).bind(&now).execute(database.pool()).await;
            assert!(duplicate.is_err(), "same source hash and extraction version must not upload twice");
            let remote_id = Uuid::new_v4();
            sqlx::query("INSERT INTO provider_remote_resources (id, book_id, extraction_id, provider_profile_id, provider_kind, encrypted_reference, cleanup_status, created_at, updated_at) VALUES (?, ?, ?, ?, 'kimi', 'enc:v1:fixture', 'pending', ?, ?)")
                .bind(remote_id.to_string()).bind(book_id.to_string()).bind(extraction_id.to_string()).bind(profile_id.to_string()).bind(&now).bind(&now).execute(database.pool()).await.unwrap();
            let guarded = sqlx::query("DELETE FROM books WHERE id = ?").bind(book_id.to_string()).execute(database.pool()).await;
            assert!(guarded.is_err(), "remote cleanup must be detached before book deletion");
            sqlx::query("UPDATE provider_remote_resources SET book_id = NULL WHERE id = ?").bind(remote_id.to_string()).execute(database.pool()).await.unwrap();
            sqlx::query("DELETE FROM books WHERE id = ?").bind(book_id.to_string()).execute(database.pool()).await.unwrap();
            let derived: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM book_extractions WHERE book_id = ?").bind(book_id.to_string()).fetch_one(database.pool()).await.unwrap();
            assert_eq!(derived, 0, "book deletion must cascade local extracted text");
            let detached = sqlx::query("SELECT book_id, extraction_id FROM provider_remote_resources WHERE id = ?").bind(remote_id.to_string()).fetch_one(database.pool()).await.unwrap();
            assert!(detached.try_get::<Option<String>, _>("book_id").unwrap().is_none());
            assert!(detached.try_get::<Option<String>, _>("extraction_id").unwrap().is_none());
        });
    }

    #[test]
    fn cancellation_registry_rejects_duplicate_start_and_cancels_in_flight() {
        let registry = ExtractionCancellationRegistry::default();
        let book = Uuid::new_v4();
        let token = registry.begin(book).unwrap();
        assert!(registry.begin(book).is_err());
        assert!(registry.cancel(book));
        assert!(token.is_cancelled());
        registry.finish(book);
        assert!(registry.begin(book).is_ok());
    }

    #[test]
    fn cancellation_during_upload_is_durable_cancelled_not_failed() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("app-data");
        let paths = AppPaths {
            books: root.join("books"),
            cache: root.join("cache"),
            logs: root.join("logs"),
            database: root.join("library.sqlite3"),
            root,
        };
        for path in [&paths.root, &paths.books, &paths.cache, &paths.logs] {
            fs::create_dir_all(path).unwrap();
        }
        let database = Database::open(&paths.database).unwrap();
        tauri::async_runtime::block_on(async {
            let book_id = Uuid::new_v4();
            let profile_id = Uuid::new_v4();
            let timestamp = database_timestamp(Utc::now());
            let bytes = b"%PDF synthetic cancellable extraction";
            let source_hash = hex(Sha256::digest(bytes));
            let book_directory = paths.books.join(book_id.to_string());
            fs::create_dir_all(&book_directory).unwrap();
            fs::write(book_directory.join("original.pdf"), bytes).unwrap();
            sqlx::query("INSERT INTO books (id, sha256, title, format, original_filename, stored_path, import_status, created_at, updated_at) VALUES (?, ?, 'Cancellation fixture', 'pdf', 'fixture.pdf', ?, 'ready', ?, ?)")
                .bind(book_id.to_string()).bind(&source_hash)
                .bind(format!("books/{book_id}/original.pdf"))
                .bind(&timestamp).bind(&timestamp).execute(database.pool()).await.unwrap();
            sqlx::query("INSERT INTO provider_profiles (id, provider_kind, display_name, model_id, context_window_tokens, created_at, updated_at, validated_at, kimi_api_region) VALUES (?, 'kimi', 'Cancellation Kimi', 'kimi-k3', 1000000, ?, ?, ?, 'cn')")
                .bind(profile_id.to_string()).bind(&timestamp).bind(&timestamp).bind(&timestamp)
                .execute(database.pool()).await.unwrap();
            let credentials = Arc::new(MemoryCredentialStore::new());
            credentials
                .set(
                    &credential_key(profile_id),
                    SecretString::from("synthetic-cancel-upload-credential"),
                )
                .await
                .unwrap();
            let server = MockServer::start().await;
            let upload_started = Arc::new(Notify::new());
            let responder_signal = upload_started.clone();
            Mock::given(method("POST"))
                .and(path("/v1/files"))
                .respond_with(move |_: &Request| {
                    responder_signal.notify_one();
                    ResponseTemplate::new(200)
                        .set_delay(Duration::from_millis(250))
                        .set_body_json(
                            serde_json::json!({"id":"file_cancel_fixture","status":"uploaded"}),
                        )
                })
                .expect(1)
                .mount(&server)
                .await;
            let client = KimiFilesClient::new_for_test_region(
                KimiApiRegion::Cn,
                &format!("{}/v1", server.uri()),
            )
            .unwrap();
            let cancel = CancellationToken::new();
            let task_cancel = cancel.clone();
            let pool = database.pool().clone();
            let task_paths = paths.clone();
            let task_credentials = credentials.clone();
            let task = tokio::spawn(async move {
                prepare(
                    PrepareDependencies {
                        pool: &pool,
                        paths: &task_paths,
                        credentials: task_credentials,
                        capabilities: ProviderCapabilityRegistry::load_embedded().unwrap(),
                        client: &client,
                    },
                    book_id,
                    profile_id,
                    task_cancel,
                )
                .await
            });
            tokio::time::timeout(Duration::from_secs(5), upload_started.notified())
                .await
                .expect("synthetic upload request did not reach the loopback server");
            cancel.cancel();
            let summary = task.await.unwrap().unwrap();
            assert_eq!(summary.status, "cancelled");
            let durable_status: String =
                sqlx::query_scalar("SELECT status FROM book_extractions WHERE id = ?")
                    .bind(summary.id.to_string())
                    .fetch_one(database.pool())
                    .await
                    .unwrap();
            assert_eq!(durable_status, "cancelled");
            let remote_count: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM provider_remote_resources WHERE extraction_id = ?",
            )
            .bind(summary.id.to_string())
            .fetch_one(database.pool())
            .await
            .unwrap();
            assert_eq!(remote_count, 0);
        });
    }
}
