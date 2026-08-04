use std::{
    collections::{BTreeSet, HashMap},
    fmt, fs,
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

use async_trait::async_trait;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{Row, SqlitePool};
use tokio::sync::{Mutex as AsyncMutex, Semaphore};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{
    ai::{
        multimodal::stage_vision_asset,
        registry::ProviderCapabilityRegistry,
        runtime::ProviderRuntime,
        structured::{MAX_PROVIDER_PAGE_ANALYSIS_BYTES, PAGE_ANALYSIS_SCHEMA_VERSION},
    },
    app_state::AppPaths,
    credentials::CredentialStore,
    db::{
        indexing::{self, CreateIndexRun},
        providers,
    },
    documents::source::read_book_source_bytes,
    domain::{
        AiOperation, CapabilitySupport, DocumentLocator, ImageLimits, ImageMime, IndexFailureCode,
        IndexPageStatus, IndexQualityReason, ProviderKind, ProviderOperationConsent,
        ProviderPageAnalysis, RemoteCleanupHandle, StructuredAnalysisOutcome,
        StructuredPageRequest,
    },
    errors::{AppError, AppErrorCode, AppResult},
    indexing::{
        commit::{PageCommitRequest, commit_validated_page},
        recovery::scratch_path,
        remote_cleanup, state,
        validator::{RequestedPageValidation, validate_batch},
    },
};

pub const INDEX_RENDER_VERSION: &str = "pdfjs-render-v1";
pub const INDEX_PARSER_VERSION: &str = "pdfjs-layout-v1";
pub const MAX_COORDINATOR_BATCH_PAGES: usize = 2;
pub const MAX_INDEX_OPERATION_PAGES: usize = 10_000;
const MAX_ACTIVE_RENDER_OR_SEND_PAGES: i64 = 4;
const PROVIDER_SLOT_TIMEOUT: Duration = Duration::from_secs(30);
const PROVIDER_TIMEOUT: Duration = Duration::from_secs(90);
const REMOTE_COMPENSATION_TIMEOUT: Duration = Duration::from_secs(30);
const APPLICATION_MAX_ENCODED_BYTES_EACH: u64 = 4 * 1024 * 1024;
const APPLICATION_MAX_TOTAL_ENCODED_BYTES: u64 = 12 * 1024 * 1024;
const APPLICATION_MAX_DIMENSION_PX: u32 = 4_096;
const APPLICATION_MAX_DECODED_PIXELS_EACH: u64 = 8_847_360;
const MAX_PENDING_OPERATION_TOKENS: usize = 64;
const MAX_AUTHORIZED_RUNS: usize = 64;
const MAX_RETRY_IDEMPOTENCY_RECORDS: usize = 4_096;
const RETRY_LOCK_STRIPES: usize = 64;
const OPERATION_TOKEN_TTL: Duration = Duration::from_secs(10 * 60);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IndexPageSeed {
    pub page_number: u32,
    pub quality_reason: IndexQualityReason,
    pub local_text_sha256: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConfirmIndexOperationRequest {
    pub run_id: Option<Uuid>,
    pub book_id: Uuid,
    pub source_sha256: String,
    pub provider_profile_id: Uuid,
    pub pages: Vec<IndexPageSeed>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct OperationBinding {
    target_run_id: Option<Uuid>,
    book_id: Uuid,
    source_sha256: String,
    provider_profile_id: Uuid,
    provider_kind: ProviderKind,
    model_id: String,
    pages: Vec<IndexPageSeed>,
    consent_fingerprint: String,
}

#[derive(Default)]
struct OperationState {
    pending: HashMap<Uuid, PendingOperation>,
    authorized_runs: HashMap<Uuid, OperationBinding>,
    active_claims: HashMap<(Uuid, Uuid, Uuid), CancellationToken>,
    retry_results: HashMap<(Uuid, String), Uuid>,
}

struct PendingOperation {
    binding: OperationBinding,
    issued_at: Instant,
}

#[derive(Clone)]
pub struct IndexOperationRegistry {
    state: Arc<Mutex<OperationState>>,
    provider_slots: Arc<Semaphore>,
    retry_locks: Arc<Vec<AsyncMutex<()>>>,
}

impl Default for IndexOperationRegistry {
    fn default() -> Self {
        Self {
            state: Arc::new(Mutex::new(OperationState::default())),
            provider_slots: Arc::new(Semaphore::new(MAX_COORDINATOR_BATCH_PAGES)),
            retry_locks: Arc::new(
                (0..RETRY_LOCK_STRIPES)
                    .map(|_| AsyncMutex::new(()))
                    .collect(),
            ),
        }
    }
}

impl fmt::Debug for IndexOperationRegistry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = self.state.lock();
        formatter
            .debug_struct("IndexOperationRegistry")
            .field("pending_count", &state.pending.len())
            .field("authorized_run_count", &state.authorized_runs.len())
            .finish()
    }
}

impl IndexOperationRegistry {
    fn issue(&self, binding: OperationBinding) -> AppResult<Uuid> {
        let mut state = self.state.lock();
        state
            .pending
            .retain(|_, pending| pending.issued_at.elapsed() <= OPERATION_TOKEN_TTL);
        if state.pending.len() >= MAX_PENDING_OPERATION_TOKENS {
            return Err(AppError::new(AppErrorCode::RequestConflict));
        }
        let token = Uuid::new_v4();
        state.pending.insert(
            token,
            PendingOperation {
                binding,
                issued_at: Instant::now(),
            },
        );
        Ok(token)
    }

    fn consume(&self, token: Uuid, expected: &OperationBinding) -> AppResult<OperationBinding> {
        let pending = self
            .state
            .lock()
            .pending
            .remove(&token)
            .ok_or_else(|| AppError::new(AppErrorCode::RequestConflict))?;
        if pending.issued_at.elapsed() > OPERATION_TOKEN_TTL {
            return Err(AppError::new(AppErrorCode::RequestConflict));
        }
        let actual = pending.binding;
        if &actual != expected {
            return Err(AppError::new(AppErrorCode::InvalidInput));
        }
        Ok(actual)
    }

    fn authorize(&self, run_id: Uuid, binding: OperationBinding) -> AppResult<()> {
        let mut state = self.state.lock();
        if !state.authorized_runs.contains_key(&run_id)
            && state.authorized_runs.len() >= MAX_AUTHORIZED_RUNS
        {
            return Err(AppError::new(AppErrorCode::RequestConflict));
        }
        state.authorized_runs.insert(run_id, binding);
        Ok(())
    }

    fn authorization(&self, run_id: Uuid) -> Option<OperationBinding> {
        self.state.lock().authorized_runs.get(&run_id).cloned()
    }

    fn authorized_run_ids(&self) -> Vec<Uuid> {
        let mut ids = self
            .state
            .lock()
            .authorized_runs
            .keys()
            .copied()
            .collect::<Vec<_>>();
        ids.sort_unstable();
        ids
    }

    pub fn revoke_run(&self, run_id: Uuid) {
        self.state.lock().authorized_runs.remove(&run_id);
    }

    fn register_claim(&self, run_id: Uuid, page_id: Uuid, attempt_id: Uuid) -> AppResult<()> {
        let mut state = self.state.lock();
        if state
            .active_claims
            .keys()
            .any(|(_, candidate_page, _)| *candidate_page == page_id)
        {
            return Err(AppError::new(AppErrorCode::RequestConflict));
        }
        state
            .active_claims
            .insert((run_id, page_id, attempt_id), CancellationToken::new());
        Ok(())
    }

    fn claim_token(
        &self,
        run_id: Uuid,
        page_id: Uuid,
        attempt_id: Uuid,
    ) -> AppResult<CancellationToken> {
        self.state
            .lock()
            .active_claims
            .get(&(run_id, page_id, attempt_id))
            .cloned()
            .ok_or_else(|| AppError::new(AppErrorCode::RequestConflict))
    }

    fn finish_claim(&self, run_id: Uuid, page_id: Uuid, attempt_id: Uuid) {
        self.state
            .lock()
            .active_claims
            .remove(&(run_id, page_id, attempt_id));
    }

    fn cancel_claims(&self, run_id: Uuid) {
        for ((candidate_run, _, _), token) in &self.state.lock().active_claims {
            if *candidate_run == run_id {
                token.cancel();
            }
        }
    }

    fn retry_lock(&self, page_id: Uuid) -> &AsyncMutex<()> {
        let index = (page_id.as_u128() % RETRY_LOCK_STRIPES as u128) as usize;
        &self.retry_locks[index]
    }

    fn retry_result(&self, page_id: Uuid, expected_updated_at: &str) -> Option<Uuid> {
        self.state
            .lock()
            .retry_results
            .get(&(page_id, expected_updated_at.to_owned()))
            .copied()
    }

    fn forget_retry(&self, page_id: Uuid, expected_updated_at: &str) {
        self.state
            .lock()
            .retry_results
            .remove(&(page_id, expected_updated_at.to_owned()));
    }

    fn remember_retry(&self, page_id: Uuid, expected_updated_at: &str, new_attempt_id: Uuid) {
        let mut state = self.state.lock();
        state
            .retry_results
            .retain(|(candidate_page_id, _), _| *candidate_page_id != page_id);
        if state.retry_results.len() >= MAX_RETRY_IDEMPOTENCY_RECORDS
            && let Some(key) = state.retry_results.keys().next().cloned()
        {
            state.retry_results.remove(&key);
        }
        state
            .retry_results
            .insert((page_id, expected_updated_at.to_owned()), new_attempt_id);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RenderLimitsDto {
    pub max_dimension: u32,
    pub max_decoded_pixels: u64,
    pub max_encoded_bytes: u64,
    pub max_total_encoded_bytes: u64,
}

impl From<ImageLimits> for RenderLimitsDto {
    fn from(value: ImageLimits) -> Self {
        Self {
            max_dimension: value.max_dimension_px,
            max_decoded_pixels: value.max_decoded_pixels_each,
            max_encoded_bytes: value.max_encoded_bytes_each,
            max_total_encoded_bytes: value.max_total_encoded_bytes,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RenderClaimDto {
    pub run_id: Uuid,
    pub book_id: Uuid,
    pub page_id: Uuid,
    pub page_number: u32,
    pub attempt_id: Uuid,
    pub limits: RenderLimitsDto,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RenderClaimBatchDto {
    pub claims: Vec<RenderClaimDto>,
}

pub struct RenderedPageSubmission {
    pub run_id: Uuid,
    pub book_id: Uuid,
    pub page_id: Uuid,
    pub page_number: u32,
    pub attempt_id: Uuid,
    pub schema_version: u16,
    pub mime_type: String,
    pub width: u32,
    pub height: u32,
    pub decoded_pixel_count: u64,
    pub encoded_byte_length: u64,
    pub sha256: String,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RenderedPageCaptureMetadata {
    pub run_id: Uuid,
    pub book_id: Uuid,
    pub page_id: Uuid,
    pub page_number: u32,
    pub attempt_id: Uuid,
    pub schema_version: u16,
    pub mime_type: String,
    pub width: u32,
    pub height: u32,
    pub decoded_pixel_count: u64,
    pub encoded_byte_length: u64,
    pub sha256: String,
    pub byte_offset: u64,
}

impl fmt::Debug for RenderedPageCaptureMetadata {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RenderedPageCaptureMetadata")
            .field("run_id", &self.run_id)
            .field("book_id", &self.book_id)
            .field("page_id", &self.page_id)
            .field("page_number", &self.page_number)
            .field("attempt_id", &self.attempt_id)
            .field("schema_version", &self.schema_version)
            .field("mime_type", &self.mime_type)
            .field("width", &self.width)
            .field("height", &self.height)
            .field("decoded_pixel_count", &self.decoded_pixel_count)
            .field("encoded_byte_length", &self.encoded_byte_length)
            .field("sha256", &self.sha256)
            .field("byte_offset", &self.byte_offset)
            .finish()
    }
}

impl fmt::Debug for RenderedPageSubmission {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RenderedPageSubmission")
            .field("run_id", &self.run_id)
            .field("book_id", &self.book_id)
            .field("page_id", &self.page_id)
            .field("page_number", &self.page_number)
            .field("attempt_id", &self.attempt_id)
            .field("schema_version", &self.schema_version)
            .field("mime_type", &self.mime_type)
            .field("width", &self.width)
            .field("height", &self.height)
            .field("decoded_pixel_count", &self.decoded_pixel_count)
            .field("encoded_byte_length", &self.encoded_byte_length)
            .field("sha256", &self.sha256)
            .field("bytes", &"[REDACTED]")
            .finish()
    }
}

pub fn decode_rendered_submissions(
    metadata: Vec<RenderedPageCaptureMetadata>,
    body: &[u8],
) -> AppResult<Vec<RenderedPageSubmission>> {
    if metadata.is_empty()
        || metadata.len() > MAX_COORDINATOR_BATCH_PAGES
        || body.is_empty()
        || u64::try_from(body.len()).map_err(|_| invalid_input())?
            > APPLICATION_MAX_TOTAL_ENCODED_BYTES
    {
        return Err(invalid_input());
    }
    let mut expected_offset = 0_u64;
    let mut page_ids = BTreeSet::new();
    let mut submissions = Vec::with_capacity(metadata.len());
    for page in metadata {
        if page.byte_offset != expected_offset
            || page.encoded_byte_length == 0
            || page.encoded_byte_length > APPLICATION_MAX_ENCODED_BYTES_EACH
            || !page_ids.insert(page.page_id)
        {
            return Err(invalid_input());
        }
        let end = expected_offset
            .checked_add(page.encoded_byte_length)
            .ok_or_else(invalid_input)?;
        let start_index = usize::try_from(expected_offset).map_err(|_| invalid_input())?;
        let end_index = usize::try_from(end).map_err(|_| invalid_input())?;
        let bytes = body
            .get(start_index..end_index)
            .ok_or_else(invalid_input)?
            .to_vec();
        submissions.push(RenderedPageSubmission {
            run_id: page.run_id,
            book_id: page.book_id,
            page_id: page.page_id,
            page_number: page.page_number,
            attempt_id: page.attempt_id,
            schema_version: page.schema_version,
            mime_type: page.mime_type,
            width: page.width,
            height: page.height,
            decoded_pixel_count: page.decoded_pixel_count,
            encoded_byte_length: page.encoded_byte_length,
            sha256: page.sha256,
            bytes,
        });
        expected_offset = end;
    }
    if expected_offset != u64::try_from(body.len()).map_err(|_| invalid_input())? {
        return Err(invalid_input());
    }
    Ok(submissions)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexingEventDto {
    pub run_id: Uuid,
    pub page_id: Uuid,
    pub status: IndexPageStatus,
    pub safe_error_code: Option<IndexFailureCode>,
}

/// Rust-only continuation value. Provider content is deliberately not serializable.
pub struct ReceivedAnalysis {
    pub analysis: Option<ProviderPageAnalysis>,
    pub events: Vec<IndexingEventDto>,
}

impl fmt::Debug for ReceivedAnalysis {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ReceivedAnalysis")
            .field("has_analysis", &self.analysis.is_some())
            .field("event_count", &self.events.len())
            .finish()
    }
}

#[async_trait]
pub trait AnalysisExecutor: Send + Sync {
    async fn analyze(
        &self,
        pool: &SqlitePool,
        profile_id: Uuid,
        request: StructuredPageRequest,
        cancel: CancellationToken,
    ) -> AppResult<StructuredAnalysisOutcome>;

    async fn cleanup(
        &self,
        pool: &SqlitePool,
        profile_id: Uuid,
        handle: &RemoteCleanupHandle,
        cancel: CancellationToken,
    ) -> AppResult<()>;
}

struct RuntimeAnalysisExecutor {
    credential_store: Arc<dyn CredentialStore>,
    capabilities: ProviderCapabilityRegistry,
}

#[async_trait]
impl AnalysisExecutor for RuntimeAnalysisExecutor {
    async fn analyze(
        &self,
        pool: &SqlitePool,
        profile_id: Uuid,
        request: StructuredPageRequest,
        cancel: CancellationToken,
    ) -> AppResult<StructuredAnalysisOutcome> {
        ProviderRuntime::new(self.credential_store.clone(), self.capabilities.clone())
            .load(pool, profile_id, AiOperation::StructuredPageAnalysis)
            .await?
            .analyze_pages(request, cancel)
            .await
    }

    async fn cleanup(
        &self,
        pool: &SqlitePool,
        profile_id: Uuid,
        handle: &RemoteCleanupHandle,
        cancel: CancellationToken,
    ) -> AppResult<()> {
        ProviderRuntime::new(self.credential_store.clone(), self.capabilities.clone())
            .load(pool, profile_id, AiOperation::StructuredPageAnalysis)
            .await?
            .cleanup_remote_resource(handle, cancel)
            .await
    }
}

pub struct IndexCoordinatorService {
    pool: SqlitePool,
    paths: AppPaths,
    operations: IndexOperationRegistry,
    cancellations: state::IndexCancellationRegistry,
    capabilities: ProviderCapabilityRegistry,
    credential_store: Arc<dyn CredentialStore>,
    executor: Arc<dyn AnalysisExecutor>,
}

impl IndexCoordinatorService {
    pub fn new(
        pool: SqlitePool,
        paths: AppPaths,
        operations: IndexOperationRegistry,
        cancellations: state::IndexCancellationRegistry,
        capabilities: ProviderCapabilityRegistry,
        credential_store: Arc<dyn CredentialStore>,
    ) -> Self {
        let executor = Arc::new(RuntimeAnalysisExecutor {
            credential_store: credential_store.clone(),
            capabilities: capabilities.clone(),
        });
        Self {
            pool,
            paths,
            operations,
            cancellations,
            capabilities,
            credential_store,
            executor,
        }
    }

    #[cfg(test)]
    pub(crate) fn with_executor(mut self, executor: Arc<dyn AnalysisExecutor>) -> Self {
        self.executor = executor;
        self
    }

    pub async fn confirm_operation(
        &self,
        request: ConfirmIndexOperationRequest,
    ) -> AppResult<Uuid> {
        let binding = validated_binding(&self.pool, &self.capabilities, request).await?;
        self.operations.issue(binding)
    }

    pub async fn create_run(
        &self,
        operation_token: Uuid,
        request: ConfirmIndexOperationRequest,
    ) -> AppResult<Uuid> {
        if request.run_id.is_some() {
            return Err(AppError::new(AppErrorCode::InvalidInput));
        }
        let current = validated_binding(&self.pool, &self.capabilities, request).await?;
        let binding = self.operations.consume(operation_token, &current)?;
        let run_id = indexing::create_index_run(
            &self.pool,
            CreateIndexRun {
                book_id: binding.book_id,
                provider_profile_id: binding.provider_profile_id,
                analysis_schema_version: PAGE_ANALYSIS_SCHEMA_VERSION.to_owned(),
                render_version: INDEX_RENDER_VERSION.to_owned(),
                parser_version: INDEX_PARSER_VERSION.to_owned(),
            },
        )
        .await?;
        for page in &binding.pages {
            if let Err(error) = state::queue(
                &self.pool,
                run_id,
                page.page_number,
                page.quality_reason,
                page.local_text_sha256.clone(),
            )
            .await
            {
                self.operations.revoke_run(run_id);
                let _ = sqlx::query("DELETE FROM index_runs WHERE id = ?")
                    .bind(run_id.to_string())
                    .execute(&self.pool)
                    .await;
                return Err(error);
            }
        }
        if let Err(error) = self.operations.authorize(run_id, binding) {
            let _ = sqlx::query("DELETE FROM index_runs WHERE id = ?")
                .bind(run_id.to_string())
                .execute(&self.pool)
                .await;
            return Err(error);
        }
        Ok(run_id)
    }

    pub async fn authorize_run(
        &self,
        run_id: Uuid,
        operation_token: Uuid,
        request: ConfirmIndexOperationRequest,
    ) -> AppResult<()> {
        if request.run_id != Some(run_id) {
            return Err(AppError::new(AppErrorCode::InvalidInput));
        }
        let current = validated_binding(&self.pool, &self.capabilities, request).await?;
        assert_run_matches_binding(&self.pool, run_id, &current).await?;
        let binding = self.operations.consume(operation_token, &current)?;
        self.operations.authorize(run_id, binding)
    }

    pub async fn claim_render_batch(&self) -> AppResult<RenderClaimBatchDto> {
        let active: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM index_pages WHERE status IN ('rendering', 'sending')",
        )
        .fetch_one(&self.pool)
        .await?;
        if active >= MAX_ACTIVE_RENDER_OR_SEND_PAGES {
            return Ok(RenderClaimBatchDto { claims: Vec::new() });
        }
        let available = usize::try_from(MAX_ACTIVE_RENDER_OR_SEND_PAGES - active)
            .map_err(|_| database_error())?;

        for run_id in self.operations.authorized_run_ids() {
            let binding = match recheck_authorization(
                &self.pool,
                &self.capabilities,
                &self.operations,
                run_id,
            )
            .await
            {
                Ok(binding) => binding,
                Err(_) => continue,
            };
            let limits = image_limits(
                &self.capabilities,
                &binding.provider_kind,
                &binding.model_id,
            )?;
            let batch_limit = MAX_COORDINATOR_BATCH_PAGES
                .min(usize::from(limits.max_images))
                .min(available);
            if batch_limit == 0 {
                return Ok(RenderClaimBatchDto { claims: Vec::new() });
            }
            let rows = sqlx::query(
                "SELECT id, page_number, attempt_id FROM index_pages WHERE run_id = ? AND status = 'queued' ORDER BY page_number LIMIT ?",
            )
            .bind(run_id.to_string())
            .bind(i64::try_from(batch_limit).map_err(|_| database_error())?)
            .fetch_all(&self.pool)
            .await?;
            let mut claims = Vec::with_capacity(rows.len());
            for row in rows {
                let page_id = parse_uuid(&row.try_get::<String, _>("id")?)?;
                let attempt_id = parse_uuid(&row.try_get::<String, _>("attempt_id")?)?;
                let page_number = u32::try_from(row.try_get::<i64, _>("page_number")?)
                    .map_err(|_| database_error())?;
                if state::claim_render(&self.pool, page_id, attempt_id)
                    .await
                    .is_ok()
                {
                    if self
                        .operations
                        .register_claim(run_id, page_id, attempt_id)
                        .is_err()
                    {
                        let _ = state::fail(
                            &self.pool,
                            page_id,
                            IndexPageStatus::Rendering,
                            attempt_id,
                            IndexFailureCode::IndexAttemptInterrupted,
                            true,
                        )
                        .await;
                        continue;
                    }
                    claims.push(RenderClaimDto {
                        run_id,
                        book_id: binding.book_id,
                        page_id,
                        page_number,
                        attempt_id,
                        limits: limits.into(),
                    });
                }
            }
            if !claims.is_empty() {
                return Ok(RenderClaimBatchDto { claims });
            }
        }
        Ok(RenderClaimBatchDto { claims: Vec::new() })
    }

    pub async fn read_claimed_source(&self, page_id: Uuid, attempt_id: Uuid) -> AppResult<Vec<u8>> {
        let page =
            load_page_attempt(&self.pool, page_id, attempt_id, IndexPageStatus::Rendering).await?;
        let binding = recheck_authorization(
            &self.pool,
            &self.capabilities,
            &self.operations,
            page.run_id,
        )
        .await?;
        if page.book_id != binding.book_id || !binding.pages.contains(&page.seed) {
            return Err(AppError::new(AppErrorCode::RequestConflict));
        }
        let bytes = read_book_source_bytes(&self.pool, &self.paths, page.book_id).await?;
        if sha256_hex(&bytes) != binding.source_sha256 {
            self.operations.revoke_run(page.run_id);
            return Err(AppError::new(AppErrorCode::RequestConflict));
        }
        load_page_attempt(&self.pool, page_id, attempt_id, IndexPageStatus::Rendering).await?;
        Ok(bytes)
    }

    pub async fn submit_rendered_batch(
        &self,
        submissions: Vec<RenderedPageSubmission>,
    ) -> AppResult<ReceivedAnalysis> {
        submit_with_executor(self, submissions, self.executor.clone()).await
    }

    pub async fn report_render_failure(
        &self,
        page_id: Uuid,
        attempt_id: Uuid,
    ) -> AppResult<IndexingEventDto> {
        let page =
            load_page_attempt(&self.pool, page_id, attempt_id, IndexPageStatus::Rendering).await?;
        let claim = self
            .operations
            .claim_token(page.run_id, page_id, attempt_id)?;
        if load_run_status(&self.pool, page.run_id).await?
            == crate::domain::IndexRunStatus::Cancelling
            && claim.is_cancelled()
        {
            state::cancel(&self.pool, page_id, IndexPageStatus::Rendering, attempt_id).await?;
            self.operations
                .finish_claim(page.run_id, page_id, attempt_id);
            remove_scratch(&self.paths.indexing_scratch(), page_id, attempt_id);
            finalize_cancel_if_possible(&self.pool, page.run_id).await;
            return Ok(IndexingEventDto {
                run_id: page.run_id,
                page_id,
                status: IndexPageStatus::Cancelled,
                safe_error_code: None,
            });
        }
        state::fail(
            &self.pool,
            page_id,
            IndexPageStatus::Rendering,
            attempt_id,
            IndexFailureCode::IndexRenderFailed,
            true,
        )
        .await?;
        self.operations
            .finish_claim(page.run_id, page_id, attempt_id);
        remove_scratch(&self.paths.indexing_scratch(), page_id, attempt_id);
        Ok(failed_event(
            page.run_id,
            page_id,
            IndexFailureCode::IndexRenderFailed,
        ))
    }

    pub async fn pause_run(&self, run_id: Uuid) -> AppResult<()> {
        let status = load_run_status(&self.pool, run_id).await?;
        match status {
            crate::domain::IndexRunStatus::Paused => Ok(()),
            crate::domain::IndexRunStatus::Running => {
                match state::pause(&self.pool, run_id, status).await {
                    Ok(()) => Ok(()),
                    Err(error)
                        if error.code == AppErrorCode::RequestConflict
                            && load_run_status(&self.pool, run_id).await?
                                == crate::domain::IndexRunStatus::Paused =>
                    {
                        Ok(())
                    }
                    Err(error) => Err(error),
                }
            }
            _ => Err(AppError::new(AppErrorCode::RequestConflict)),
        }
    }

    pub async fn resume_run(&self, run_id: Uuid) -> AppResult<()> {
        let status = load_run_status(&self.pool, run_id).await?;
        match status {
            crate::domain::IndexRunStatus::Running => Ok(()),
            crate::domain::IndexRunStatus::Paused => {
                match state::resume(&self.pool, run_id, status).await {
                    Ok(()) => Ok(()),
                    Err(error)
                        if error.code == AppErrorCode::RequestConflict
                            && load_run_status(&self.pool, run_id).await?
                                == crate::domain::IndexRunStatus::Running =>
                    {
                        Ok(())
                    }
                    Err(error) => Err(error),
                }
            }
            _ => Err(AppError::new(AppErrorCode::RequestConflict)),
        }
    }

    pub async fn cancel_run(&self, run_id: Uuid) -> AppResult<()> {
        let status = load_run_status(&self.pool, run_id).await?;
        match status {
            crate::domain::IndexRunStatus::Cancelling
            | crate::domain::IndexRunStatus::Cancelled => Ok(()),
            crate::domain::IndexRunStatus::Running | crate::domain::IndexRunStatus::Paused => {
                self.operations.cancel_claims(run_id);
                match state::cancel_run(&self.pool, &self.cancellations, run_id, status).await {
                    Ok(_) => {
                        self.operations.revoke_run(run_id);
                        finalize_cancel_if_possible(&self.pool, run_id).await;
                        Ok(())
                    }
                    Err(error)
                        if error.code == AppErrorCode::RequestConflict
                            && matches!(
                                load_run_status(&self.pool, run_id).await?,
                                crate::domain::IndexRunStatus::Cancelling
                                    | crate::domain::IndexRunStatus::Cancelled
                            ) =>
                    {
                        self.operations.revoke_run(run_id);
                        finalize_cancel_if_possible(&self.pool, run_id).await;
                        Ok(())
                    }
                    Err(error) => Err(error),
                }
            }
            crate::domain::IndexRunStatus::Completed => {
                Err(AppError::new(AppErrorCode::RequestConflict))
            }
        }
    }

    pub async fn retry_page(&self, page_id: Uuid, expected_updated_at: &str) -> AppResult<Uuid> {
        validate_retry_version(expected_updated_at)?;
        let _retry_guard = self.operations.retry_lock(page_id).lock().await;
        let row = sqlx::query(
            "SELECT run_id, status, attempt_id, updated_at FROM index_pages WHERE id = ?",
        )
        .bind(page_id.to_string())
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| AppError::new(AppErrorCode::NotFound))?;
        let run_id = parse_uuid(&row.try_get::<String, _>("run_id")?)?;
        let current_attempt = parse_uuid(&row.try_get::<String, _>("attempt_id")?)?;
        let status = IndexPageStatus::from_database(&row.try_get::<String, _>("status")?)
            .ok_or_else(database_error)?;
        let current_updated_at = row.try_get::<String, _>("updated_at")?;
        indexing::parse_timestamp(&current_updated_at)?;

        if let Some(result) = self.operations.retry_result(page_id, expected_updated_at) {
            if status == IndexPageStatus::Queued && current_attempt == result {
                return Ok(result);
            }
            self.operations.forget_retry(page_id, expected_updated_at);
            return Err(AppError::new(AppErrorCode::RequestConflict));
        }

        if current_updated_at != expected_updated_at
            || !matches!(
                status,
                IndexPageStatus::Failed | IndexPageStatus::NeedsReview
            )
        {
            return Err(AppError::new(AppErrorCode::RequestConflict));
        }

        let new_attempt = state::retry(&self.pool, page_id, status, current_attempt).await?;
        self.operations
            .finish_claim(run_id, page_id, current_attempt);
        self.operations
            .remember_retry(page_id, expected_updated_at, new_attempt);
        Ok(new_attempt)
    }
}

async fn submit_with_executor(
    service: &IndexCoordinatorService,
    submissions: Vec<RenderedPageSubmission>,
    executor: Arc<dyn AnalysisExecutor>,
) -> AppResult<ReceivedAnalysis> {
    if submissions.is_empty() || submissions.len() > MAX_COORDINATOR_BATCH_PAGES {
        return Err(AppError::new(AppErrorCode::InvalidInput));
    }
    let first_run_id = submissions[0].run_id;
    let first_book_id = submissions[0].book_id;
    if submissions.iter().any(|page| {
        page.run_id != first_run_id
            || page.book_id != first_book_id
            || page.page_id == Uuid::nil()
            || page.attempt_id == Uuid::nil()
    }) {
        return Err(AppError::new(AppErrorCode::InvalidInput));
    }
    let unique_pages = submissions
        .iter()
        .map(|page| page.page_id)
        .collect::<BTreeSet<_>>();
    if unique_pages.len() != submissions.len() {
        return Err(AppError::new(AppErrorCode::InvalidInput));
    }

    if load_run_status(&service.pool, first_run_id).await?
        == crate::domain::IndexRunStatus::Cancelling
    {
        let mut events = Vec::with_capacity(submissions.len());
        for submission in submissions {
            let page = load_page_attempt(
                &service.pool,
                submission.page_id,
                submission.attempt_id,
                IndexPageStatus::Rendering,
            )
            .await?;
            let cancel = service.operations.claim_token(
                submission.run_id,
                submission.page_id,
                submission.attempt_id,
            )?;
            if page.run_id != submission.run_id
                || page.book_id != submission.book_id
                || page.page_number != submission.page_number
                || !cancel.is_cancelled()
            {
                return Err(AppError::new(AppErrorCode::RequestConflict));
            }
            state::cancel(
                &service.pool,
                submission.page_id,
                IndexPageStatus::Rendering,
                submission.attempt_id,
            )
            .await?;
            service.operations.finish_claim(
                submission.run_id,
                submission.page_id,
                submission.attempt_id,
            );
            remove_scratch(
                &service.paths.indexing_scratch(),
                submission.page_id,
                submission.attempt_id,
            );
            events.push(IndexingEventDto {
                run_id: submission.run_id,
                page_id: submission.page_id,
                status: IndexPageStatus::Cancelled,
                safe_error_code: None,
            });
        }
        finalize_cancel_if_possible(&service.pool, first_run_id).await;
        return Ok(ReceivedAnalysis {
            analysis: None,
            events,
        });
    }

    assert_owned_rendering_submissions(service, &submissions).await?;

    let binding = match recheck_authorization(
        &service.pool,
        &service.capabilities,
        &service.operations,
        first_run_id,
    )
    .await
    {
        Ok(binding) => binding,
        Err(_) => {
            return finish_interrupted_rendering_batch(service, submissions).await;
        }
    };
    if binding.book_id != first_book_id {
        return Err(AppError::new(AppErrorCode::RequestConflict));
    }
    let limits = image_limits(
        &service.capabilities,
        &binding.provider_kind,
        &binding.model_id,
    )?;
    if submissions.len() > usize::from(limits.max_images) {
        return Err(AppError::new(AppErrorCode::InvalidInput));
    }
    for submission in &submissions {
        let page = load_page_attempt(
            &service.pool,
            submission.page_id,
            submission.attempt_id,
            IndexPageStatus::Rendering,
        )
        .await?;
        if page.run_id != submission.run_id
            || page.book_id != submission.book_id
            || page.page_number != submission.page_number
            || !binding.pages.contains(&page.seed)
        {
            return Err(AppError::new(AppErrorCode::RequestConflict));
        }
    }
    let queue_cancel = service.operations.claim_token(
        submissions[0].run_id,
        submissions[0].page_id,
        submissions[0].attempt_id,
    )?;
    let provider_slot = tokio::time::timeout(
        PROVIDER_SLOT_TIMEOUT,
        service.operations.provider_slots.clone().acquire_owned(),
    );
    let _provider_slot = tokio::select! {
        () = queue_cancel.cancelled() => {
            return finish_interrupted_rendering_batch(service, submissions).await;
        }
        result = provider_slot => match result {
            Ok(Ok(permit)) => permit,
            Ok(Err(_)) | Err(_) => {
                return finish_interrupted_rendering_batch(service, submissions).await;
            }
        },
    };

    let mut assets = Vec::with_capacity(submissions.len());
    let mut active = Vec::with_capacity(submissions.len());
    let mut events = Vec::with_capacity(submissions.len());
    let mut total_encoded = 0_u64;
    for submission in submissions {
        let page = load_page_attempt(
            &service.pool,
            submission.page_id,
            submission.attempt_id,
            IndexPageStatus::Rendering,
        )
        .await?;
        if page.run_id != submission.run_id
            || page.book_id != submission.book_id
            || page.page_number != submission.page_number
            || !binding.pages.contains(&page.seed)
        {
            return Err(AppError::new(AppErrorCode::RequestConflict));
        }
        let local_valid = submission.schema_version == 1
            && submission.mime_type == "image/png"
            && submission.width > 0
            && submission.height > 0
            && submission.decoded_pixel_count
                == u64::from(submission.width) * u64::from(submission.height)
            && submission.encoded_byte_length
                == u64::try_from(submission.bytes.len()).map_err(|_| invalid_input())?
            && submission.sha256 == sha256_hex(&submission.bytes);
        let next_total_encoded = total_encoded
            .checked_add(submission.encoded_byte_length)
            .ok_or_else(invalid_input)?;
        if !local_valid || next_total_encoded > limits.max_total_encoded_bytes {
            let _ = state::fail(
                &service.pool,
                submission.page_id,
                IndexPageStatus::Rendering,
                submission.attempt_id,
                IndexFailureCode::IndexRenderFailed,
                true,
            )
            .await;
            service.operations.finish_claim(
                submission.run_id,
                submission.page_id,
                submission.attempt_id,
            );
            events.push(failed_event(
                submission.run_id,
                submission.page_id,
                IndexFailureCode::IndexRenderFailed,
            ));
            continue;
        }

        let cancel = service.operations.claim_token(
            submission.run_id,
            submission.page_id,
            submission.attempt_id,
        )?;
        if cancel.is_cancelled() {
            let _ = state::cancel(
                &service.pool,
                submission.page_id,
                IndexPageStatus::Rendering,
                submission.attempt_id,
            )
            .await;
            service.operations.finish_claim(
                submission.run_id,
                submission.page_id,
                submission.attempt_id,
            );
            remove_scratch(
                &service.paths.indexing_scratch(),
                submission.page_id,
                submission.attempt_id,
            );
            events.push(IndexingEventDto {
                run_id: submission.run_id,
                page_id: submission.page_id,
                status: IndexPageStatus::Cancelled,
                safe_error_code: None,
            });
            continue;
        }
        if write_render_scratch(
            &service.paths.indexing_scratch(),
            submission.page_id,
            submission.attempt_id,
            &submission.bytes,
        )
        .is_err()
        {
            service.operations.finish_claim(
                submission.run_id,
                submission.page_id,
                submission.attempt_id,
            );
            let _ = state::fail(
                &service.pool,
                submission.page_id,
                IndexPageStatus::Rendering,
                submission.attempt_id,
                IndexFailureCode::IndexRenderFailed,
                true,
            )
            .await;
            events.push(failed_event(
                submission.run_id,
                submission.page_id,
                IndexFailureCode::IndexRenderFailed,
            ));
            continue;
        }
        load_page_attempt(
            &service.pool,
            submission.page_id,
            submission.attempt_id,
            IndexPageStatus::Rendering,
        )
        .await?;
        let asset = match stage_vision_asset(
            submission.book_id,
            submission.page_id,
            ImageMime::Png,
            submission.width,
            submission.height,
            submission.bytes,
            limits,
        ) {
            Ok(asset) => asset,
            Err(_) => {
                remove_scratch(
                    &service.paths.indexing_scratch(),
                    submission.page_id,
                    submission.attempt_id,
                );
                service.operations.finish_claim(
                    submission.run_id,
                    submission.page_id,
                    submission.attempt_id,
                );
                let _ = state::fail(
                    &service.pool,
                    submission.page_id,
                    IndexPageStatus::Rendering,
                    submission.attempt_id,
                    IndexFailureCode::IndexRenderFailed,
                    true,
                )
                .await;
                events.push(failed_event(
                    submission.run_id,
                    submission.page_id,
                    IndexFailureCode::IndexRenderFailed,
                ));
                continue;
            }
        };
        total_encoded = next_total_encoded;
        if cancel.is_cancelled() {
            let _ = state::cancel(
                &service.pool,
                submission.page_id,
                IndexPageStatus::Rendering,
                submission.attempt_id,
            )
            .await;
            service.operations.finish_claim(
                submission.run_id,
                submission.page_id,
                submission.attempt_id,
            );
            remove_scratch(
                &service.paths.indexing_scratch(),
                submission.page_id,
                submission.attempt_id,
            );
            events.push(IndexingEventDto {
                run_id: submission.run_id,
                page_id: submission.page_id,
                status: IndexPageStatus::Cancelled,
                safe_error_code: None,
            });
            continue;
        }
        state::mark_rendered(
            &service.pool,
            submission.page_id,
            submission.attempt_id,
            &submission.sha256,
        )
        .await?;
        state::claim_send(&service.pool, submission.page_id, submission.attempt_id).await?;
        active.push((
            submission.run_id,
            submission.page_id,
            submission.attempt_id,
            cancel,
        ));
        assets.push(asset);
    }

    if assets.is_empty() {
        return Ok(ReceivedAnalysis {
            analysis: None,
            events,
        });
    }
    if active.iter().any(|(_, _, _, cancel)| cancel.is_cancelled()) {
        return finish_cancelled_batch(service, active, events).await;
    }
    if recheck_authorization(
        &service.pool,
        &service.capabilities,
        &service.operations,
        first_run_id,
    )
    .await
    .is_err()
    {
        return fail_active_batch(
            service,
            active,
            events,
            IndexFailureCode::IndexAttemptInterrupted,
        )
        .await;
    }
    let mut owns_all_sending_attempts = true;
    for (_, page_id, attempt_id, _) in &active {
        if load_page_attempt(
            &service.pool,
            *page_id,
            *attempt_id,
            IndexPageStatus::Sending,
        )
        .await
        .is_err()
        {
            owns_all_sending_attempts = false;
            break;
        }
    }
    if !owns_all_sending_attempts {
        return fail_active_batch(
            service,
            active,
            events,
            IndexFailureCode::IndexAttemptInterrupted,
        )
        .await;
    }

    let provider_cancel = active[0].3.clone();
    let request = StructuredPageRequest {
        model: binding.model_id.clone(),
        pages: assets,
        schema_version: PAGE_ANALYSIS_SCHEMA_VERSION.to_owned(),
        max_output_bytes: MAX_PROVIDER_PAGE_ANALYSIS_BYTES,
    };
    let outcome = tokio::time::timeout(
        PROVIDER_TIMEOUT,
        executor.analyze(
            &service.pool,
            binding.provider_profile_id,
            request,
            provider_cancel,
        ),
    )
    .await;
    let mut outcome = match outcome {
        Ok(Ok(outcome)) => outcome,
        Ok(Err(error)) if error.code == AppErrorCode::ImportCancelled => {
            return finish_cancelled_batch(service, active, events).await;
        }
        Ok(Err(_)) | Err(_) => {
            return fail_active_batch(
                service,
                active,
                events,
                IndexFailureCode::IndexProviderFailed,
            )
            .await;
        }
    };

    if active.iter().any(|(_, _, _, cancel)| cancel.is_cancelled()) {
        return finish_cancelled_batch(service, active, events).await;
    }
    if recheck_authorization(
        &service.pool,
        &service.capabilities,
        &service.operations,
        first_run_id,
    )
    .await
    .is_err()
    {
        return fail_active_batch(
            service,
            active,
            events,
            IndexFailureCode::IndexAttemptInterrupted,
        )
        .await;
    }
    let mut owns_all_sending_attempts = true;
    for (_, page_id, attempt_id, _) in &active {
        if load_page_attempt(
            &service.pool,
            *page_id,
            *attempt_id,
            IndexPageStatus::Sending,
        )
        .await
        .is_err()
        {
            owns_all_sending_attempts = false;
            break;
        }
    }
    if !owns_all_sending_attempts {
        return fail_active_batch(
            service,
            active,
            events,
            IndexFailureCode::IndexAttemptInterrupted,
        )
        .await;
    }
    let response_bytes = serde_json::to_vec(&outcome.analysis).map_err(AppError::database)?;
    if response_bytes.len()
        > usize::try_from(MAX_PROVIDER_PAGE_ANALYSIS_BYTES).map_err(|_| invalid_input())?
    {
        return fail_active_batch(
            service,
            active,
            events,
            IndexFailureCode::IndexResponseInvalid,
        )
        .await;
    }
    let response_sha256 = sha256_hex(&response_bytes);
    for (run_id, page_id, attempt_id, cancel) in &active {
        if cancel.is_cancelled() {
            let _ = state::cancel(
                &service.pool,
                *page_id,
                IndexPageStatus::Sending,
                *attempt_id,
            )
            .await;
            events.push(IndexingEventDto {
                run_id: *run_id,
                page_id: *page_id,
                status: IndexPageStatus::Cancelled,
                safe_error_code: None,
            });
            remove_scratch(&service.paths.indexing_scratch(), *page_id, *attempt_id);
        } else {
            load_page_attempt(
                &service.pool,
                *page_id,
                *attempt_id,
                IndexPageStatus::Sending,
            )
            .await?;
            state::mark_received(&service.pool, *page_id, *attempt_id, &response_sha256).await?;
            remove_scratch(&service.paths.indexing_scratch(), *page_id, *attempt_id);
            load_page_attempt(
                &service.pool,
                *page_id,
                *attempt_id,
                IndexPageStatus::Parsing,
            )
            .await?;
            events.push(IndexingEventDto {
                run_id: *run_id,
                page_id: *page_id,
                status: IndexPageStatus::Parsing,
                safe_error_code: None,
            });
        }
    }

    if let Some(handle) = outcome.cleanup.take() {
        let tracked = remote_cleanup::store_remote_handle(
            &service.pool,
            service.credential_store.as_ref(),
            active[0].1,
            &handle,
        )
        .await;
        if let Err(failure) = tracked {
            let pool = service.pool.clone();
            let credential_store = service.credential_store.clone();
            let profile_id = binding.provider_profile_id;
            tauri::async_runtime::spawn(async move {
                let compensation = tokio::time::timeout(
                    REMOTE_COMPENSATION_TIMEOUT,
                    executor.cleanup(&pool, profile_id, &handle, CancellationToken::new()),
                )
                .await
                .is_ok_and(|result| result.is_ok());
                remote_cleanup::finish_failed_tracking_compensation(
                    credential_store.as_ref(),
                    failure,
                    compensation,
                )
                .await;
            });
        }
    }

    validate_and_commit_received(
        service,
        active,
        outcome.analysis,
        response_bytes.len(),
        events,
    )
    .await
}

async fn validate_and_commit_received(
    service: &IndexCoordinatorService,
    active: Vec<(Uuid, Uuid, Uuid, CancellationToken)>,
    analysis: ProviderPageAnalysis,
    response_bytes: usize,
    mut events: Vec<IndexingEventDto>,
) -> AppResult<ReceivedAnalysis> {
    let run_id = active
        .first()
        .map(|(run_id, _, _, _)| *run_id)
        .ok_or_else(invalid_input)?;

    // Cancellation barrier: after receipt but before local semantic validation.
    for (_, page_id, attempt_id, cancel) in &active {
        if cancel.is_cancelled()
            && page_attempt_status(&service.pool, *page_id, *attempt_id).await?
                == IndexPageStatus::Parsing
        {
            state::cancel(
                &service.pool,
                *page_id,
                IndexPageStatus::Parsing,
                *attempt_id,
            )
            .await?;
            update_event(&mut events, *page_id, IndexPageStatus::Cancelled, None);
        }
    }

    let mut requested_pages = Vec::with_capacity(active.len());
    for (_, page_id, attempt_id, _) in &active {
        let (book_id, page_number) =
            load_page_identity(&service.pool, *page_id, *attempt_id).await?;
        requested_pages.push(RequestedPageValidation {
            page_number,
            local_text: load_local_page_text(&service.pool, book_id, page_number).await?,
        });
    }
    let validation = match validate_batch(&analysis, response_bytes, &requested_pages) {
        Ok(validation) => validation,
        Err(_) => {
            for (_, page_id, attempt_id, _) in &active {
                if page_attempt_status(&service.pool, *page_id, *attempt_id).await?
                    == IndexPageStatus::Parsing
                {
                    state::fail(
                        &service.pool,
                        *page_id,
                        IndexPageStatus::Parsing,
                        *attempt_id,
                        IndexFailureCode::IndexResponseInvalid,
                        true,
                    )
                    .await?;
                    update_event(
                        &mut events,
                        *page_id,
                        IndexPageStatus::Failed,
                        Some(IndexFailureCode::IndexResponseInvalid),
                    );
                }
            }
            finish_received_claims(service, &active).await;
            finalize_terminal_run(&service.pool, run_id).await;
            return Ok(ReceivedAnalysis {
                analysis: None,
                events,
            });
        }
    };
    let validation_by_page = validation
        .into_iter()
        .map(|page| (page.page_number, page.result))
        .collect::<HashMap<_, _>>();

    for (event_run_id, page_id, attempt_id, cancel) in &active {
        let status = page_attempt_status(&service.pool, *page_id, *attempt_id).await?;
        if status == IndexPageStatus::Cancelled {
            continue;
        }
        if status != IndexPageStatus::Parsing {
            return Err(AppError::new(AppErrorCode::RequestConflict));
        }
        if cancel.is_cancelled() {
            state::cancel(
                &service.pool,
                *page_id,
                IndexPageStatus::Parsing,
                *attempt_id,
            )
            .await?;
            update_event(&mut events, *page_id, IndexPageStatus::Cancelled, None);
            continue;
        }
        let (_, page_number) = load_page_identity(&service.pool, *page_id, *attempt_id).await?;
        let Some(result) = validation_by_page.get(&page_number) else {
            return Err(AppError::new(AppErrorCode::RequestConflict));
        };
        let page = match result {
            Ok(page) => page,
            Err(_) => {
                state::fail(
                    &service.pool,
                    *page_id,
                    IndexPageStatus::Parsing,
                    *attempt_id,
                    IndexFailureCode::IndexValidationFailed,
                    true,
                )
                .await?;
                update_event(
                    &mut events,
                    *page_id,
                    IndexPageStatus::Failed,
                    Some(IndexFailureCode::IndexValidationFailed),
                );
                continue;
            }
        };
        match commit_validated_page(
            &service.pool,
            PageCommitRequest {
                page_id: *page_id,
                attempt_id: *attempt_id,
                page,
            },
            cancel,
        )
        .await
        {
            Ok(outcome) => update_event(&mut events, *page_id, outcome.status, None),
            Err(error) if error.code == AppErrorCode::ImportCancelled => {
                state::cancel(
                    &service.pool,
                    *page_id,
                    IndexPageStatus::Parsing,
                    *attempt_id,
                )
                .await?;
                update_event(&mut events, *page_id, IndexPageStatus::Cancelled, None);
            }
            Err(_) => {
                let current = page_attempt_status(&service.pool, *page_id, *attempt_id).await?;
                if current != IndexPageStatus::Parsing {
                    return Err(AppError::new(AppErrorCode::RequestConflict));
                }
                state::fail(
                    &service.pool,
                    *page_id,
                    IndexPageStatus::Parsing,
                    *attempt_id,
                    IndexFailureCode::IndexValidationFailed,
                    true,
                )
                .await?;
                update_event(
                    &mut events,
                    *page_id,
                    IndexPageStatus::Failed,
                    Some(IndexFailureCode::IndexValidationFailed),
                );
            }
        }
        debug_assert_eq!(*event_run_id, run_id);
    }

    finish_received_claims(service, &active).await;
    finalize_terminal_run(&service.pool, run_id).await;
    Ok(ReceivedAnalysis {
        analysis: None,
        events,
    })
}

async fn finish_received_claims(
    service: &IndexCoordinatorService,
    active: &[(Uuid, Uuid, Uuid, CancellationToken)],
) {
    for (run_id, page_id, attempt_id, _) in active {
        service
            .operations
            .finish_claim(*run_id, *page_id, *attempt_id);
    }
}

async fn finalize_terminal_run(pool: &SqlitePool, run_id: Uuid) {
    if let Ok(status) = load_run_status(pool, run_id).await {
        let _ = state::finalize_run_if_terminal(pool, run_id, status).await;
    }
}

fn update_event(
    events: &mut [IndexingEventDto],
    page_id: Uuid,
    status: IndexPageStatus,
    safe_error_code: Option<IndexFailureCode>,
) {
    if let Some(event) = events.iter_mut().find(|event| event.page_id == page_id) {
        event.status = status;
        event.safe_error_code = safe_error_code;
    }
}

async fn load_page_identity(
    pool: &SqlitePool,
    page_id: Uuid,
    attempt_id: Uuid,
) -> AppResult<(Uuid, u32)> {
    let row =
        sqlx::query("SELECT book_id, page_number FROM index_pages WHERE id = ? AND attempt_id = ?")
            .bind(page_id.to_string())
            .bind(attempt_id.to_string())
            .fetch_optional(pool)
            .await?
            .ok_or_else(|| AppError::new(AppErrorCode::RequestConflict))?;
    Ok((
        parse_uuid(&row.try_get::<String, _>("book_id")?)?,
        u32::try_from(row.try_get::<i64, _>("page_number")?).map_err(|_| database_error())?,
    ))
}

async fn load_local_page_text(
    pool: &SqlitePool,
    book_id: Uuid,
    page_number: u32,
) -> AppResult<Option<String>> {
    const MAX_LOCAL_COMPARISON_CODE_POINTS: usize = 262_144;
    let rows = sqlx::query(
        "SELECT plain_text, locator_json FROM blocks WHERE book_id = ? ORDER BY section_id, ordinal",
    )
    .bind(book_id.to_string())
    .fetch_all(pool)
    .await?;
    let mut text = String::new();
    for row in rows {
        let locator: DocumentLocator =
            serde_json::from_str(&row.try_get::<String, _>("locator_json")?)
                .map_err(AppError::database)?;
        let belongs_to_page = matches!(
            locator,
            DocumentLocator::Pdf {
                start_page,
                end_page,
                ..
            } if (start_page..=end_page).contains(&page_number)
        );
        if !belongs_to_page {
            continue;
        }
        let block_text: String = row.try_get("plain_text")?;
        let next_count = text
            .chars()
            .count()
            .saturating_add(block_text.chars().count());
        if next_count > MAX_LOCAL_COMPARISON_CODE_POINTS {
            return Ok(None);
        }
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(&block_text);
    }
    Ok((!text.trim().is_empty()).then_some(text))
}

async fn assert_owned_rendering_submissions(
    service: &IndexCoordinatorService,
    submissions: &[RenderedPageSubmission],
) -> AppResult<()> {
    for submission in submissions {
        let page = load_page_attempt(
            &service.pool,
            submission.page_id,
            submission.attempt_id,
            IndexPageStatus::Rendering,
        )
        .await?;
        service.operations.claim_token(
            submission.run_id,
            submission.page_id,
            submission.attempt_id,
        )?;
        if page.run_id != submission.run_id
            || page.book_id != submission.book_id
            || page.page_number != submission.page_number
        {
            return Err(AppError::new(AppErrorCode::RequestConflict));
        }
    }
    Ok(())
}

async fn finish_interrupted_rendering_batch(
    service: &IndexCoordinatorService,
    submissions: Vec<RenderedPageSubmission>,
) -> AppResult<ReceivedAnalysis> {
    let run_id = submissions.first().map(|submission| submission.run_id);
    let mut events = Vec::with_capacity(submissions.len());
    for submission in submissions {
        let cancel = service.operations.claim_token(
            submission.run_id,
            submission.page_id,
            submission.attempt_id,
        )?;
        let cancelling = cancel.is_cancelled()
            || load_run_status(&service.pool, submission.run_id).await?
                == crate::domain::IndexRunStatus::Cancelling;
        if cancelling {
            state::cancel(
                &service.pool,
                submission.page_id,
                IndexPageStatus::Rendering,
                submission.attempt_id,
            )
            .await?;
            events.push(IndexingEventDto {
                run_id: submission.run_id,
                page_id: submission.page_id,
                status: IndexPageStatus::Cancelled,
                safe_error_code: None,
            });
        } else {
            state::fail(
                &service.pool,
                submission.page_id,
                IndexPageStatus::Rendering,
                submission.attempt_id,
                IndexFailureCode::IndexAttemptInterrupted,
                true,
            )
            .await?;
            events.push(failed_event(
                submission.run_id,
                submission.page_id,
                IndexFailureCode::IndexAttemptInterrupted,
            ));
        }
        service.operations.finish_claim(
            submission.run_id,
            submission.page_id,
            submission.attempt_id,
        );
        remove_scratch(
            &service.paths.indexing_scratch(),
            submission.page_id,
            submission.attempt_id,
        );
    }
    if let Some(run_id) = run_id {
        finalize_cancel_if_possible(&service.pool, run_id).await;
    }
    Ok(ReceivedAnalysis {
        analysis: None,
        events,
    })
}

async fn finish_cancelled_batch(
    service: &IndexCoordinatorService,
    active: Vec<(Uuid, Uuid, Uuid, CancellationToken)>,
    mut events: Vec<IndexingEventDto>,
) -> AppResult<ReceivedAnalysis> {
    let run_id = active.first().map(|active| active.0);
    for (run_id, page_id, attempt_id, _) in active {
        let _ = state::cancel(&service.pool, page_id, IndexPageStatus::Sending, attempt_id).await;
        service.operations.finish_claim(run_id, page_id, attempt_id);
        remove_scratch(&service.paths.indexing_scratch(), page_id, attempt_id);
        events.push(IndexingEventDto {
            run_id,
            page_id,
            status: IndexPageStatus::Cancelled,
            safe_error_code: None,
        });
    }
    if let Some(run_id) = run_id {
        finalize_cancel_if_possible(&service.pool, run_id).await;
    }
    Ok(ReceivedAnalysis {
        analysis: None,
        events,
    })
}

async fn fail_active_batch(
    service: &IndexCoordinatorService,
    active: Vec<(Uuid, Uuid, Uuid, CancellationToken)>,
    mut events: Vec<IndexingEventDto>,
    failure: IndexFailureCode,
) -> AppResult<ReceivedAnalysis> {
    for (run_id, page_id, attempt_id, _) in active {
        let _ = state::fail(
            &service.pool,
            page_id,
            IndexPageStatus::Sending,
            attempt_id,
            failure,
            true,
        )
        .await;
        service.operations.finish_claim(run_id, page_id, attempt_id);
        remove_scratch(&service.paths.indexing_scratch(), page_id, attempt_id);
        events.push(failed_event(run_id, page_id, failure));
    }
    Ok(ReceivedAnalysis {
        analysis: None,
        events,
    })
}

#[derive(Debug)]
struct PageAttempt {
    run_id: Uuid,
    book_id: Uuid,
    page_number: u32,
    seed: IndexPageSeed,
}

async fn load_page_attempt(
    pool: &SqlitePool,
    page_id: Uuid,
    attempt_id: Uuid,
    expected_status: IndexPageStatus,
) -> AppResult<PageAttempt> {
    let row = sqlx::query(
        "SELECT run_id, book_id, page_number, quality_reason, local_text_sha256 FROM index_pages WHERE id = ? AND attempt_id = ? AND status = ?",
    )
    .bind(page_id.to_string())
    .bind(attempt_id.to_string())
    .bind(expected_status.as_str())
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::new(AppErrorCode::RequestConflict))?;
    let quality = IndexQualityReason::from_database(&row.try_get::<String, _>("quality_reason")?)
        .ok_or_else(database_error)?;
    let page_number =
        u32::try_from(row.try_get::<i64, _>("page_number")?).map_err(|_| database_error())?;
    Ok(PageAttempt {
        run_id: parse_uuid(&row.try_get::<String, _>("run_id")?)?,
        book_id: parse_uuid(&row.try_get::<String, _>("book_id")?)?,
        page_number,
        seed: IndexPageSeed {
            page_number,
            quality_reason: quality,
            local_text_sha256: row.try_get("local_text_sha256")?,
        },
    })
}

async fn page_attempt_status(
    pool: &SqlitePool,
    page_id: Uuid,
    attempt_id: Uuid,
) -> AppResult<IndexPageStatus> {
    let status: String =
        sqlx::query_scalar("SELECT status FROM index_pages WHERE id = ? AND attempt_id = ?")
            .bind(page_id.to_string())
            .bind(attempt_id.to_string())
            .fetch_optional(pool)
            .await?
            .ok_or_else(|| AppError::new(AppErrorCode::RequestConflict))?;
    IndexPageStatus::from_database(&status).ok_or_else(database_error)
}

async fn finalize_cancel_if_possible(pool: &SqlitePool, run_id: Uuid) {
    if load_run_status(pool, run_id).await.ok() == Some(crate::domain::IndexRunStatus::Cancelling) {
        let _ = state::finalize_run_if_terminal(
            pool,
            run_id,
            crate::domain::IndexRunStatus::Cancelling,
        )
        .await;
    }
}

async fn validated_binding(
    pool: &SqlitePool,
    capabilities: &ProviderCapabilityRegistry,
    request: ConfirmIndexOperationRequest,
) -> AppResult<OperationBinding> {
    validate_sha256(&request.source_sha256)?;
    let pages = canonical_pages(request.pages)?;
    let book = sqlx::query("SELECT sha256, format, import_status FROM books WHERE id = ?")
        .bind(request.book_id.to_string())
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| AppError::new(AppErrorCode::NotFound))?;
    if book.try_get::<String, _>("format")? != "pdf"
        || book.try_get::<String, _>("import_status")? != "ready"
        || book.try_get::<Option<String>, _>("sha256")?.as_deref()
            != Some(request.source_sha256.as_str())
    {
        return Err(AppError::new(AppErrorCode::RequestConflict));
    }
    let profile = sqlx::query("SELECT provider_kind, model_id FROM provider_profiles WHERE id = ?")
        .bind(request.provider_profile_id.to_string())
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| AppError::new(AppErrorCode::NotFound))?;
    let provider_kind = parse_provider_kind(&profile.try_get::<String, _>("provider_kind")?)?;
    let model_id: String = profile.try_get("model_id")?;
    if capabilities.operation_support(
        &provider_kind,
        &model_id,
        AiOperation::StructuredPageAnalysis,
    ) != CapabilitySupport::Supported
    {
        return Err(AppError::unsupported_provider_capability());
    }
    image_limits(capabilities, &provider_kind, &model_id)?;
    let consent_fingerprint = consent_fingerprint(
        &providers::list_provider_operation_consents(pool, request.provider_profile_id).await?,
    )?;
    Ok(OperationBinding {
        target_run_id: request.run_id,
        book_id: request.book_id,
        source_sha256: request.source_sha256,
        provider_profile_id: request.provider_profile_id,
        provider_kind,
        model_id,
        pages,
        consent_fingerprint,
    })
}

async fn recheck_authorization(
    pool: &SqlitePool,
    capabilities: &ProviderCapabilityRegistry,
    operations: &IndexOperationRegistry,
    run_id: Uuid,
) -> AppResult<OperationBinding> {
    let binding = operations
        .authorization(run_id)
        .ok_or_else(|| AppError::new(AppErrorCode::RequestConflict))?;
    if assert_run_matches_binding(pool, run_id, &binding)
        .await
        .is_err()
        || capabilities.operation_support(
            &binding.provider_kind,
            &binding.model_id,
            AiOperation::StructuredPageAnalysis,
        ) != CapabilitySupport::Supported
    {
        operations.revoke_run(run_id);
        return Err(AppError::new(AppErrorCode::RequestConflict));
    }
    let current = consent_fingerprint(
        &providers::list_provider_operation_consents(pool, binding.provider_profile_id).await?,
    )?;
    if current != binding.consent_fingerprint {
        operations.revoke_run(run_id);
        return Err(AppError::new(AppErrorCode::RequestConflict));
    }
    Ok(binding)
}

async fn assert_run_matches_binding(
    pool: &SqlitePool,
    run_id: Uuid,
    binding: &OperationBinding,
) -> AppResult<()> {
    if binding.target_run_id.is_some() && binding.target_run_id != Some(run_id) {
        return Err(AppError::new(AppErrorCode::InvalidInput));
    }
    let run = sqlx::query(
        "SELECT r.book_id, r.source_sha256, r.provider_profile_id, r.provider_kind, r.model_id, r.analysis_schema_version, r.render_version, r.parser_version, r.status, b.sha256 AS current_source_sha256, b.format AS current_book_format, b.import_status AS current_book_status, p.provider_kind AS current_profile_kind, p.model_id AS current_profile_model FROM index_runs r JOIN books b ON b.id = r.book_id LEFT JOIN provider_profiles p ON p.id = r.provider_profile_id WHERE r.id = ?",
    )
    .bind(run_id.to_string())
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::new(AppErrorCode::NotFound))?;
    if run.try_get::<String, _>("book_id")? != binding.book_id.to_string()
        || run.try_get::<String, _>("source_sha256")? != binding.source_sha256
        || run
            .try_get::<Option<String>, _>("current_source_sha256")?
            .as_deref()
            != Some(binding.source_sha256.as_str())
        || run.try_get::<String, _>("current_book_format")? != "pdf"
        || run.try_get::<String, _>("current_book_status")? != "ready"
        || run
            .try_get::<Option<String>, _>("provider_profile_id")?
            .as_deref()
            != Some(binding.provider_profile_id.to_string().as_str())
        || parse_provider_kind(&run.try_get::<String, _>("provider_kind")?)?
            != binding.provider_kind
        || run.try_get::<String, _>("model_id")? != binding.model_id
        || run
            .try_get::<Option<String>, _>("current_profile_kind")?
            .as_deref()
            != Some(provider_kind_name(&binding.provider_kind))
        || run
            .try_get::<Option<String>, _>("current_profile_model")?
            .as_deref()
            != Some(binding.model_id.as_str())
        || run.try_get::<String, _>("analysis_schema_version")? != PAGE_ANALYSIS_SCHEMA_VERSION
        || run.try_get::<String, _>("render_version")? != INDEX_RENDER_VERSION
        || run.try_get::<String, _>("parser_version")? != INDEX_PARSER_VERSION
        || !matches!(
            run.try_get::<String, _>("status")?.as_str(),
            "running" | "paused"
        )
    {
        return Err(AppError::new(AppErrorCode::RequestConflict));
    }
    let rows = sqlx::query(
        "SELECT page_number, quality_reason, local_text_sha256 FROM index_pages WHERE run_id = ? ORDER BY page_number",
    )
    .bind(run_id.to_string())
    .fetch_all(pool)
    .await?;
    let actual = rows
        .into_iter()
        .map(|row| {
            Ok(IndexPageSeed {
                page_number: u32::try_from(row.try_get::<i64, _>("page_number")?)
                    .map_err(|_| database_error())?,
                quality_reason: IndexQualityReason::from_database(
                    &row.try_get::<String, _>("quality_reason")?,
                )
                .ok_or_else(database_error)?,
                local_text_sha256: row.try_get("local_text_sha256")?,
            })
        })
        .collect::<AppResult<Vec<_>>>()?;
    if actual != binding.pages {
        return Err(AppError::new(AppErrorCode::RequestConflict));
    }
    Ok(())
}

fn canonical_pages(mut pages: Vec<IndexPageSeed>) -> AppResult<Vec<IndexPageSeed>> {
    if pages.is_empty() || pages.len() > MAX_INDEX_OPERATION_PAGES {
        return Err(invalid_input());
    }
    pages.sort_by_key(|page| page.page_number);
    let mut previous = None;
    for page in &pages {
        if page.page_number == 0 || previous == Some(page.page_number) {
            return Err(invalid_input());
        }
        if let Some(hash) = &page.local_text_sha256 {
            validate_sha256(hash)?;
        }
        previous = Some(page.page_number);
    }
    Ok(pages)
}

fn consent_fingerprint(consents: &[ProviderOperationConsent]) -> AppResult<String> {
    if consents.len() != 3 {
        return Err(database_error());
    }
    let mut material = String::new();
    for consent in consents {
        material.push_str(&format!(
            "{:?}|{:?}|{};",
            consent.category,
            consent.decision,
            consent.updated_at.to_rfc3339()
        ));
    }
    Ok(sha256_hex(material.as_bytes()))
}

fn image_limits(
    capabilities: &ProviderCapabilityRegistry,
    provider_kind: &ProviderKind,
    model_id: &str,
) -> AppResult<ImageLimits> {
    let provider = capabilities
        .capabilities()
        .iter()
        .find(|provider| &provider.kind == provider_kind)
        .and_then(|provider| provider.models.iter().find(|model| model.id == model_id))
        .and_then(|model| model.image_limits)
        .ok_or_else(AppError::unsupported_provider_capability)?;
    Ok(ImageLimits {
        max_images: provider
            .max_images
            .min(u16::try_from(MAX_COORDINATOR_BATCH_PAGES).map_err(|_| invalid_input())?),
        max_encoded_bytes_each: provider
            .max_encoded_bytes_each
            .min(APPLICATION_MAX_ENCODED_BYTES_EACH),
        max_total_encoded_bytes: provider
            .max_total_encoded_bytes
            .min(APPLICATION_MAX_TOTAL_ENCODED_BYTES),
        max_dimension_px: provider.max_dimension_px.min(APPLICATION_MAX_DIMENSION_PX),
        max_decoded_pixels_each: provider
            .max_decoded_pixels_each
            .min(APPLICATION_MAX_DECODED_PIXELS_EACH),
    })
}

fn write_render_scratch(
    root: &Path,
    page_id: Uuid,
    attempt_id: Uuid,
    bytes: &[u8],
) -> AppResult<()> {
    let target = scratch_path(root, page_id, attempt_id);
    let parent = target
        .parent()
        .ok_or_else(|| AppError::new(AppErrorCode::LocalIoError))?;
    fs::create_dir_all(parent)?;
    let temporary = parent.join(format!("{attempt_id}.tmp"));
    if let Err(error) = fs::write(&temporary, bytes) {
        let _ = fs::remove_file(&temporary);
        return Err(error.into());
    }
    if let Err(error) = fs::rename(&temporary, target) {
        let _ = fs::remove_file(&temporary);
        return Err(error.into());
    }
    Ok(())
}

fn remove_scratch(root: &Path, page_id: Uuid, attempt_id: Uuid) {
    let target = scratch_path(root, page_id, attempt_id);
    let _ = fs::remove_file(&target);
    if let Some(parent) = target.parent() {
        let _ = fs::remove_file(parent.join(format!("{attempt_id}.tmp")));
    }
}

fn failed_event(run_id: Uuid, page_id: Uuid, failure: IndexFailureCode) -> IndexingEventDto {
    IndexingEventDto {
        run_id,
        page_id,
        status: IndexPageStatus::Failed,
        safe_error_code: Some(failure),
    }
}

async fn load_run_status(
    pool: &SqlitePool,
    run_id: Uuid,
) -> AppResult<crate::domain::IndexRunStatus> {
    let status: String = sqlx::query_scalar("SELECT status FROM index_runs WHERE id = ?")
        .bind(run_id.to_string())
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| AppError::new(AppErrorCode::NotFound))?;
    crate::domain::IndexRunStatus::from_database(&status).ok_or_else(database_error)
}

fn parse_provider_kind(value: &str) -> AppResult<ProviderKind> {
    match value {
        "openai" => Ok(ProviderKind::OpenAi),
        "gemini" => Ok(ProviderKind::Gemini),
        "anthropic" => Ok(ProviderKind::Anthropic),
        "deepseek" => Ok(ProviderKind::DeepSeek),
        "kimi" => Ok(ProviderKind::Kimi),
        _ => Err(database_error()),
    }
}

const fn provider_kind_name(value: &ProviderKind) -> &'static str {
    match value {
        ProviderKind::OpenAi => "openai",
        ProviderKind::Gemini => "gemini",
        ProviderKind::Anthropic => "anthropic",
        ProviderKind::DeepSeek => "deepseek",
        ProviderKind::Kimi => "kimi",
    }
}

fn parse_uuid(value: &str) -> AppResult<Uuid> {
    Uuid::parse_str(value).map_err(|_| database_error())
}

fn validate_sha256(value: &str) -> AppResult<()> {
    if value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(())
    } else {
        Err(invalid_input())
    }
}

fn validate_retry_version(value: &str) -> AppResult<()> {
    indexing::parse_timestamp(value)
        .map(|_| ())
        .map_err(|_| invalid_input())
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn invalid_input() -> AppError {
    AppError::new(AppErrorCode::InvalidInput)
}

fn database_error() -> AppError {
    AppError::new(AppErrorCode::DatabaseError)
}
