use std::{
    fs,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use async_trait::async_trait;
use secrecy::SecretString;
use sha2::{Digest, Sha256};
use sqlx::{Row, SqlitePool};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::coordinator::{
    AnalysisExecutor, ConfirmIndexOperationRequest, IndexCoordinatorService,
    IndexOperationRegistry, IndexPageSeed, RenderedPageCaptureMetadata, RenderedPageSubmission,
    decode_rendered_submissions,
};
use crate::{
    ai::{registry::ProviderCapabilityRegistry, structured::PAGE_ANALYSIS_SCHEMA_VERSION},
    app_state::AppPaths,
    credentials::{CredentialStore, MemoryCredentialStore},
    db::Database,
    domain::{
        IndexPageStatus, IndexQualityReason, ProviderKind, ProviderPageAnalysis,
        RemoteCleanupHandle, StructuredAnalysisOutcome, StructuredPageRequest,
        UntrustedPageAnalysis,
    },
    errors::{AppError, AppErrorCode, AppResult},
    indexing::state::IndexCancellationRegistry,
};

#[derive(Default)]
struct FakeExecutor {
    calls: AtomicUsize,
    cleanup_calls: AtomicUsize,
    pages: Mutex<Vec<u32>>,
    fail: bool,
    remote_handle: bool,
    block_until_cancel: bool,
    mutate_profile_before_return: bool,
    entered: Notify,
}

#[async_trait]
impl AnalysisExecutor for FakeExecutor {
    async fn analyze(
        &self,
        pool: &SqlitePool,
        profile_id: Uuid,
        request: StructuredPageRequest,
        cancel: CancellationToken,
    ) -> AppResult<StructuredAnalysisOutcome> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if cancel.is_cancelled() {
            return Err(AppError::new(AppErrorCode::ImportCancelled));
        }
        if self.block_until_cancel {
            self.entered.notify_one();
            cancel.cancelled().await;
            return Err(AppError::new(AppErrorCode::ImportCancelled));
        }
        if self.fail {
            return Err(AppError::new(AppErrorCode::ProviderUnavailable));
        }
        let configured = self.pages.lock().unwrap().clone();
        let page_numbers = if configured.is_empty() {
            (1..=u32::try_from(request.pages.len()).unwrap()).collect()
        } else {
            configured
        };
        let analysis = ProviderPageAnalysis {
            schema_version: PAGE_ANALYSIS_SCHEMA_VERSION.to_owned(),
            pages: page_numbers
                .into_iter()
                .map(|page_number| UntrustedPageAnalysis {
                    page_number,
                    blocks: Vec::new(),
                })
                .collect(),
        };
        if self.mutate_profile_before_return {
            sqlx::query(
                "UPDATE provider_profiles SET model_id = 'late-substituted-model' WHERE id = ?",
            )
            .bind(profile_id.to_string())
            .execute(pool)
            .await?;
        }
        if self.remote_handle {
            Ok(StructuredAnalysisOutcome {
                analysis,
                cleanup: Some(RemoteCleanupHandle::new(
                    ProviderKind::OpenAi,
                    SecretString::from("synthetic-remote-outcome-sentinel"),
                )),
            })
        } else {
            Ok(StructuredAnalysisOutcome::inline(analysis))
        }
    }

    async fn cleanup(
        &self,
        _pool: &SqlitePool,
        _profile_id: Uuid,
        _handle: &crate::domain::RemoteCleanupHandle,
        _cancel: CancellationToken,
    ) -> AppResult<()> {
        self.cleanup_calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[test]
fn operation_token_is_fresh_exact_and_replay_safe() {
    let fixture = Fixture::new();
    tauri::async_runtime::block_on(async {
        let executor = Arc::new(FakeExecutor::default());
        let service = fixture.service(executor);
        let request = fixture.request(None);
        let token = service.confirm_operation(request.clone()).await.unwrap();

        let mut substituted = request.clone();
        substituted.pages[0].page_number = 3;
        let error = service.create_run(token, substituted).await.unwrap_err();
        assert_eq!(error.code, AppErrorCode::InvalidInput);
        let replay = service
            .create_run(token, request.clone())
            .await
            .unwrap_err();
        assert_eq!(replay.code, AppErrorCode::RequestConflict);

        let fresh = service.confirm_operation(request.clone()).await.unwrap();
        let run_id = service.create_run(fresh, request.clone()).await.unwrap();
        assert_eq!(
            page_numbers(fixture.database.pool(), run_id).await,
            vec![1, 2]
        );
        service.pause_run(run_id).await.unwrap();
        service.pause_run(run_id).await.unwrap();
        service.resume_run(run_id).await.unwrap();
        service.resume_run(run_id).await.unwrap();
        let replay = service.create_run(fresh, request).await.unwrap_err();
        assert_eq!(replay.code, AppErrorCode::RequestConflict);
    });
}

#[test]
fn reliable_text_is_not_required_and_never_enters_a_render_claim() {
    let fixture = Fixture::new();
    tauri::async_runtime::block_on(async {
        let executor = Arc::new(FakeExecutor::default());
        let service = fixture.service(executor.clone());
        let mut request = fixture.request(None);
        request.pages[0].quality_reason = IndexQualityReason::ReliableText;
        let token = service.confirm_operation(request.clone()).await.unwrap();
        let run_id = service.create_run(token, request).await.unwrap();

        let rows = sqlx::query(
            "SELECT page_number, status FROM index_pages WHERE run_id = ? ORDER BY page_number",
        )
        .bind(run_id.to_string())
        .fetch_all(fixture.database.pool())
        .await
        .unwrap();
        assert_eq!(rows[0].get::<i64, _>("page_number"), 1);
        assert_eq!(rows[0].get::<String, _>("status"), "queued");
        assert_eq!(rows[1].get::<i64, _>("page_number"), 2);
        assert_eq!(rows[1].get::<String, _>("status"), "not_required");
        let batch = service.claim_render_batch().await.unwrap();
        assert_eq!(batch.claims.len(), 1);
        assert_eq!(batch.claims[0].page_number, 1);
        assert_eq!(executor.calls.load(Ordering::SeqCst), 0);
    });
}

#[test]
fn restarted_run_requires_a_fresh_token_bound_to_the_exact_run_and_page_set() {
    let fixture = Fixture::new();
    tauri::async_runtime::block_on(async {
        let executor = Arc::new(FakeExecutor::default());
        let initial = fixture.service(executor.clone());
        let create_request = fixture.request(None);
        let create_token = initial
            .confirm_operation(create_request.clone())
            .await
            .unwrap();
        let run_id = initial
            .create_run(create_token, create_request)
            .await
            .unwrap();

        let restarted = fixture.service(executor);
        assert!(
            restarted
                .claim_render_batch()
                .await
                .unwrap()
                .claims
                .is_empty()
        );
        let resume_request = fixture.request(Some(run_id));
        let resume_token = restarted
            .confirm_operation(resume_request.clone())
            .await
            .unwrap();
        let wrong_run = restarted
            .authorize_run(Uuid::new_v4(), resume_token, resume_request.clone())
            .await
            .unwrap_err();
        assert_eq!(wrong_run.code, AppErrorCode::InvalidInput);
        restarted
            .authorize_run(run_id, resume_token, resume_request.clone())
            .await
            .unwrap();
        assert_eq!(
            restarted.claim_render_batch().await.unwrap().claims.len(),
            2
        );
        let replay = restarted
            .authorize_run(run_id, resume_token, resume_request)
            .await
            .unwrap_err();
        assert_eq!(replay.code, AppErrorCode::RequestConflict);
    });
}

#[test]
fn rendered_submission_debug_redacts_image_bytes() {
    let claim = super::coordinator::RenderClaimDto {
        run_id: Uuid::new_v4(),
        book_id: Uuid::new_v4(),
        page_id: Uuid::new_v4(),
        page_number: 1,
        attempt_id: Uuid::new_v4(),
        limits: crate::domain::ImageLimits {
            max_images: 1,
            max_encoded_bytes_each: 1024,
            max_total_encoded_bytes: 1024,
            max_dimension_px: 100,
            max_decoded_pixels_each: 10_000,
        }
        .into(),
    };
    let submission = submission(&claim, 7);
    let debug = format!("{submission:?}");
    assert!(debug.contains("[REDACTED]"));
    assert!(!debug.contains("137, 80, 78, 71"));
}

#[test]
fn raw_ipc_capture_metadata_requires_exact_contiguous_bounded_slices() {
    let page_id = Uuid::new_v4();
    let bytes = b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0dIHDR\x00\x00\x00\x01\x00\x00\x00\x01";
    let metadata = RenderedPageCaptureMetadata {
        run_id: Uuid::new_v4(),
        book_id: Uuid::new_v4(),
        page_id,
        page_number: 1,
        attempt_id: Uuid::new_v4(),
        schema_version: 1,
        mime_type: "image/png".to_owned(),
        width: 1,
        height: 1,
        decoded_pixel_count: 1,
        encoded_byte_length: u64::try_from(bytes.len()).unwrap(),
        sha256: hex(bytes),
        byte_offset: 0,
    };
    let mut gap = metadata.clone();
    gap.byte_offset = 1;
    let decoded = decode_rendered_submissions(vec![metadata], bytes).unwrap();
    assert_eq!(decoded.len(), 1);
    assert_eq!(decoded[0].page_id, page_id);
    assert_eq!(decoded[0].bytes, bytes);

    assert_eq!(
        decode_rendered_submissions(vec![gap], bytes)
            .unwrap_err()
            .code,
        AppErrorCode::InvalidInput
    );
}

#[test]
fn claimed_local_pages_are_bounded_batched_and_stop_at_received() {
    let fixture = Fixture::new();
    tauri::async_runtime::block_on(async {
        let executor = Arc::new(FakeExecutor::default());
        *executor.pages.lock().unwrap() = vec![1, 2];
        let service = fixture.service(executor.clone());
        let request = fixture.request(None);
        let token = service.confirm_operation(request.clone()).await.unwrap();
        let run_id = service.create_run(token, request).await.unwrap();

        let batch = service.claim_render_batch().await.unwrap();
        assert_eq!(batch.claims.len(), 2);
        assert!(batch.claims.iter().all(|claim| claim.run_id == run_id));
        assert!(
            batch
                .claims
                .iter()
                .all(|claim| claim.limits.max_dimension <= 4096)
        );
        let source = service
            .read_claimed_source(batch.claims[0].page_id, batch.claims[0].attempt_id)
            .await
            .unwrap();
        assert_eq!(source, fixture.source);

        let submissions = batch
            .claims
            .iter()
            .enumerate()
            .map(|(index, claim)| submission(claim, index as u8))
            .collect();
        let received = service.submit_rendered_batch(submissions).await.unwrap();
        assert!(received.analysis.is_some());
        assert_eq!(executor.calls.load(Ordering::SeqCst), 1);
        assert_eq!(received.events.len(), 2);
        assert!(
            received
                .events
                .iter()
                .all(|event| event.status == IndexPageStatus::Parsing)
        );
        let statuses: Vec<String> = sqlx::query_scalar(
            "SELECT status FROM index_pages WHERE run_id = ? ORDER BY page_number",
        )
        .bind(run_id.to_string())
        .fetch_all(fixture.database.pool())
        .await
        .unwrap();
        assert_eq!(statuses, vec!["parsing", "parsing"]);
        for claim in &batch.claims {
            assert!(
                !crate::indexing::recovery::scratch_path(
                    &fixture.paths.indexing_scratch(),
                    claim.page_id,
                    claim.attempt_id,
                )
                .exists()
            );
        }
        let block_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM index_page_blocks")
            .fetch_one(fixture.database.pool())
            .await
            .unwrap();
        let remote_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM provider_remote_resources")
                .fetch_one(fixture.database.pool())
                .await
                .unwrap();
        assert_eq!(block_count, 0);
        assert_eq!(remote_count, 0);
    });
}

#[test]
fn consent_change_and_cancel_are_denied_before_provider_execution() {
    let fixture = Fixture::new();
    tauri::async_runtime::block_on(async {
        let executor = Arc::new(FakeExecutor::default());
        let service = fixture.service(executor.clone());
        let request = fixture.request(None);
        let token = service.confirm_operation(request.clone()).await.unwrap();
        let run_id = service.create_run(token, request).await.unwrap();
        let batch = service.claim_render_batch().await.unwrap();

        sqlx::query(
            "UPDATE provider_operation_consents SET decision = 'skip_prompt', updated_at = '2026-08-04T00:00:01Z' WHERE profile_id = ? AND category = 'ai_index'",
        )
        .bind(fixture.profile_id.to_string())
        .execute(fixture.database.pool())
        .await
        .unwrap();
        let denied = service
            .submit_rendered_batch(
                batch
                    .claims
                    .iter()
                    .enumerate()
                    .map(|(index, claim)| submission(claim, index as u8))
                    .collect(),
            )
            .await
            .unwrap();
        assert!(denied.events.iter().all(|event| {
            event.status == IndexPageStatus::Failed
                && event.safe_error_code
                    == Some(crate::domain::IndexFailureCode::IndexAttemptInterrupted)
        }));
        assert_eq!(executor.calls.load(Ordering::SeqCst), 0);

        // A fresh operation is still required even though skip_prompt is set.
        let missing_token = service
            .create_run(Uuid::new_v4(), fixture.request(None))
            .await
            .unwrap_err();
        assert_eq!(missing_token.code, AppErrorCode::RequestConflict);
        service.cancel_run(run_id).await.unwrap();
        assert_eq!(executor.calls.load(Ordering::SeqCst), 0);
    });
}

#[test]
fn captured_profile_model_is_rechecked_before_provider_execution() {
    let fixture = Fixture::new();
    tauri::async_runtime::block_on(async {
        let executor = Arc::new(FakeExecutor::default());
        let service = fixture.service(executor.clone());
        let request = fixture.request(None);
        let token = service.confirm_operation(request.clone()).await.unwrap();
        service.create_run(token, request).await.unwrap();
        let batch = service.claim_render_batch().await.unwrap();
        sqlx::query("UPDATE provider_profiles SET model_id = 'substituted-model' WHERE id = ?")
            .bind(fixture.profile_id.to_string())
            .execute(fixture.database.pool())
            .await
            .unwrap();

        let interrupted = service
            .submit_rendered_batch(
                batch
                    .claims
                    .iter()
                    .enumerate()
                    .map(|(index, claim)| submission(claim, index as u8))
                    .collect(),
            )
            .await
            .unwrap();
        assert!(interrupted.events.iter().all(|event| {
            event.status == IndexPageStatus::Failed
                && event.safe_error_code
                    == Some(crate::domain::IndexFailureCode::IndexAttemptInterrupted)
        }));
        assert_eq!(executor.calls.load(Ordering::SeqCst), 0);
    });
}

#[test]
fn provider_failure_is_page_local_retryable_and_does_not_create_content() {
    let fixture = Fixture::new();
    tauri::async_runtime::block_on(async {
        let executor = Arc::new(FakeExecutor {
            fail: true,
            ..FakeExecutor::default()
        });
        let service = fixture.service(executor);
        let request = fixture.request(None);
        let token = service.confirm_operation(request.clone()).await.unwrap();
        let run_id = service.create_run(token, request).await.unwrap();
        let batch = service.claim_render_batch().await.unwrap();
        let result = service
            .submit_rendered_batch(
                batch
                    .claims
                    .iter()
                    .enumerate()
                    .map(|(index, claim)| submission(claim, index as u8))
                    .collect(),
            )
            .await
            .unwrap();
        assert_eq!(result.events.len(), 2);
        assert!(result.events.iter().all(|event| {
            event.status == IndexPageStatus::Failed && event.safe_error_code.is_some()
        }));
        let retryable: Vec<i64> = sqlx::query_scalar(
            "SELECT retryable FROM index_pages WHERE run_id = ? ORDER BY page_number",
        )
        .bind(run_id.to_string())
        .fetch_all(fixture.database.pool())
        .await
        .unwrap();
        assert_eq!(retryable, vec![1, 1]);
        for claim in &batch.claims {
            assert!(
                !crate::indexing::recovery::scratch_path(
                    &fixture.paths.indexing_scratch(),
                    claim.page_id,
                    claim.attempt_id,
                )
                .exists()
            );
        }
        let first_retry = service
            .retry_page(batch.claims[0].page_id, batch.claims[0].attempt_id)
            .await
            .unwrap();
        let repeated_retry = service
            .retry_page(batch.claims[0].page_id, batch.claims[0].attempt_id)
            .await
            .unwrap();
        assert_eq!(first_retry, repeated_retry);
    });
}

#[test]
fn run_cancel_interrupts_the_owned_provider_attempt_and_is_idempotent() {
    let fixture = Fixture::new();
    tauri::async_runtime::block_on(async {
        let executor = Arc::new(FakeExecutor {
            block_until_cancel: true,
            ..FakeExecutor::default()
        });
        let service = fixture.service(executor.clone());
        let request = fixture.request(None);
        let token = service.confirm_operation(request.clone()).await.unwrap();
        let run_id = service.create_run(token, request).await.unwrap();
        let batch = service.claim_render_batch().await.unwrap();
        let submissions = batch
            .claims
            .iter()
            .enumerate()
            .map(|(index, claim)| submission(claim, index as u8))
            .collect();

        let submit = service.submit_rendered_batch(submissions);
        let cancel = async {
            executor.entered.notified().await;
            service.cancel_run(run_id).await.unwrap();
            service.cancel_run(run_id).await.unwrap();
        };
        let (received, ()) = tokio::join!(submit, cancel);
        let received = received.unwrap();
        assert!(
            received
                .events
                .iter()
                .all(|event| event.status == IndexPageStatus::Cancelled)
        );
        assert_eq!(executor.calls.load(Ordering::SeqCst), 1);
        let run_status: String = sqlx::query_scalar("SELECT status FROM index_runs WHERE id = ?")
            .bind(run_id.to_string())
            .fetch_one(fixture.database.pool())
            .await
            .unwrap();
        assert_eq!(run_status, "cancelled");
        for claim in &batch.claims {
            assert!(
                !crate::indexing::recovery::scratch_path(
                    &fixture.paths.indexing_scratch(),
                    claim.page_id,
                    claim.attempt_id,
                )
                .exists()
            );
        }
    });
}

#[test]
fn cancel_during_local_render_finishes_claim_without_provider_execution() {
    let fixture = Fixture::new();
    tauri::async_runtime::block_on(async {
        let executor = Arc::new(FakeExecutor::default());
        let service = fixture.service(executor.clone());
        let request = fixture.request(None);
        let token = service.confirm_operation(request.clone()).await.unwrap();
        let run_id = service.create_run(token, request).await.unwrap();
        let batch = service.claim_render_batch().await.unwrap();
        service.cancel_run(run_id).await.unwrap();
        let received = service
            .submit_rendered_batch(
                batch
                    .claims
                    .iter()
                    .enumerate()
                    .map(|(index, claim)| submission(claim, index as u8))
                    .collect(),
            )
            .await
            .unwrap();
        assert!(
            received
                .events
                .iter()
                .all(|event| event.status == IndexPageStatus::Cancelled)
        );
        assert_eq!(executor.calls.load(Ordering::SeqCst), 0);
    });
}

#[test]
fn late_profile_change_cannot_advance_received_state_or_retain_render_bytes() {
    let fixture = Fixture::new();
    tauri::async_runtime::block_on(async {
        let executor = Arc::new(FakeExecutor {
            mutate_profile_before_return: true,
            ..FakeExecutor::default()
        });
        let service = fixture.service(executor.clone());
        let request = fixture.request(None);
        let token = service.confirm_operation(request.clone()).await.unwrap();
        let run_id = service.create_run(token, request).await.unwrap();
        let batch = service.claim_render_batch().await.unwrap();
        let result = service
            .submit_rendered_batch(
                batch
                    .claims
                    .iter()
                    .enumerate()
                    .map(|(index, claim)| submission(claim, index as u8))
                    .collect(),
            )
            .await
            .unwrap();

        assert!(result.analysis.is_none());
        assert!(result.events.iter().all(|event| {
            event.status == IndexPageStatus::Failed
                && event.safe_error_code
                    == Some(crate::domain::IndexFailureCode::IndexAttemptInterrupted)
        }));
        assert_eq!(executor.calls.load(Ordering::SeqCst), 1);
        let statuses: Vec<String> = sqlx::query_scalar(
            "SELECT status FROM index_pages WHERE run_id = ? ORDER BY page_number",
        )
        .bind(run_id.to_string())
        .fetch_all(fixture.database.pool())
        .await
        .unwrap();
        assert_eq!(statuses, vec!["failed", "failed"]);
        for claim in &batch.claims {
            assert!(
                !crate::indexing::recovery::scratch_path(
                    &fixture.paths.indexing_scratch(),
                    claim.page_id,
                    claim.attempt_id,
                )
                .exists()
            );
        }
    });
}

#[test]
fn synthetic_remote_outcome_is_tracked_without_blocking_received_state() {
    let fixture = Fixture::new();
    tauri::async_runtime::block_on(async {
        let executor = Arc::new(FakeExecutor {
            remote_handle: true,
            ..FakeExecutor::default()
        });
        let service = fixture.service(executor.clone());
        let request = fixture.request(None);
        let token = service.confirm_operation(request.clone()).await.unwrap();
        let run_id = service.create_run(token, request).await.unwrap();
        let batch = service.claim_render_batch().await.unwrap();
        let received = service
            .submit_rendered_batch(
                batch
                    .claims
                    .iter()
                    .enumerate()
                    .map(|(index, claim)| submission(claim, index as u8))
                    .collect(),
            )
            .await
            .unwrap();
        assert!(received.analysis.is_some());
        assert!(
            received
                .events
                .iter()
                .all(|event| event.status == IndexPageStatus::Parsing)
        );
        let row = sqlx::query(
            "SELECT encrypted_reference, cleanup_status FROM provider_remote_resources WHERE run_id = ?",
        )
        .bind(run_id.to_string())
        .fetch_one(fixture.database.pool())
        .await
        .unwrap();
        let reference: String = row.get("encrypted_reference");
        assert!(reference.starts_with("enc:v1:keyring:"));
        assert!(!reference.contains("synthetic-remote-outcome-sentinel"));
        assert_eq!(row.get::<String, _>("cleanup_status"), "pending");
        assert_eq!(executor.cleanup_calls.load(Ordering::SeqCst), 0);
    });
}

#[test]
fn synthetic_tracking_db_failure_calls_compensating_delete_after_received() {
    let fixture = Fixture::new();
    tauri::async_runtime::block_on(async {
        sqlx::query(
            "CREATE TRIGGER synthetic_coordinator_remote_failure BEFORE INSERT ON provider_remote_resources BEGIN SELECT RAISE(ABORT, 'synthetic coordinator remote insert failure'); END",
        )
        .execute(fixture.database.pool())
        .await
        .unwrap();
        let executor = Arc::new(FakeExecutor {
            remote_handle: true,
            ..FakeExecutor::default()
        });
        let service = fixture.service(executor.clone());
        let request = fixture.request(None);
        let token = service.confirm_operation(request.clone()).await.unwrap();
        service.create_run(token, request).await.unwrap();
        let batch = service.claim_render_batch().await.unwrap();
        let received = service
            .submit_rendered_batch(
                batch
                    .claims
                    .iter()
                    .enumerate()
                    .map(|(index, claim)| submission(claim, index as u8))
                    .collect(),
            )
            .await
            .unwrap();
        assert!(
            received
                .events
                .iter()
                .all(|event| event.status == IndexPageStatus::Parsing)
        );
        for _ in 0..100 {
            if executor.cleanup_calls.load(Ordering::SeqCst) == 1 {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert_eq!(executor.cleanup_calls.load(Ordering::SeqCst), 1);
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM provider_remote_resources")
            .fetch_one(fixture.database.pool())
            .await
            .unwrap();
        assert_eq!(count, 0);
    });
}

struct Fixture {
    _temporary: tempfile::TempDir,
    database: Database,
    paths: AppPaths,
    book_id: Uuid,
    profile_id: Uuid,
    source: Vec<u8>,
    source_sha256: String,
    store: Arc<MemoryCredentialStore>,
}

impl Fixture {
    fn new() -> Self {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("app-data");
        let paths = AppPaths {
            books: root.join("books"),
            cache: root.join("cache"),
            logs: root.join("logs"),
            database: root.join("library.sqlite3"),
            root: root.clone(),
        };
        for path in [&paths.root, &paths.books, &paths.cache, &paths.logs] {
            fs::create_dir_all(path).unwrap();
        }
        let database = Database::open(&paths.database).unwrap();
        let book_id = Uuid::new_v4();
        let profile_id = Uuid::new_v4();
        let source = b"%PDF-1.7\nSynthetic Task 4 fixture only\n%%EOF".to_vec();
        let source_sha256 = hex(&source);
        let book_dir = paths.books.join(book_id.to_string());
        fs::create_dir_all(&book_dir).unwrap();
        fs::write(book_dir.join("original.pdf"), &source).unwrap();
        tauri::async_runtime::block_on(insert_fixture(
            database.pool(),
            book_id,
            profile_id,
            &source_sha256,
        ));
        Self {
            _temporary: temporary,
            database,
            paths,
            book_id,
            profile_id,
            source,
            source_sha256,
            store: Arc::new(MemoryCredentialStore::new()),
        }
    }

    fn request(&self, run_id: Option<Uuid>) -> ConfirmIndexOperationRequest {
        ConfirmIndexOperationRequest {
            run_id,
            book_id: self.book_id,
            source_sha256: self.source_sha256.clone(),
            provider_profile_id: self.profile_id,
            pages: vec![
                IndexPageSeed {
                    page_number: 2,
                    quality_reason: IndexQualityReason::LayoutContradiction,
                    local_text_sha256: Some("b".repeat(64)),
                },
                IndexPageSeed {
                    page_number: 1,
                    quality_reason: IndexQualityReason::NoText,
                    local_text_sha256: None,
                },
            ],
        }
    }

    fn service(&self, executor: Arc<dyn AnalysisExecutor>) -> IndexCoordinatorService {
        let store: Arc<dyn CredentialStore> = self.store.clone();
        IndexCoordinatorService::new(
            self.database.pool().clone(),
            self.paths.clone(),
            IndexOperationRegistry::default(),
            IndexCancellationRegistry::default(),
            ProviderCapabilityRegistry::load_embedded().unwrap(),
            store,
        )
        .with_executor(executor)
    }
}

async fn insert_fixture(pool: &SqlitePool, book_id: Uuid, profile_id: Uuid, source_sha256: &str) {
    let timestamp = "2026-08-04T00:00:00.000Z";
    sqlx::query(
        "INSERT INTO books (id, sha256, title, format, original_filename, stored_path, import_status, created_at, updated_at) VALUES (?, ?, 'Synthetic coordinator book', 'pdf', 'synthetic.pdf', ?, 'ready', ?, ?)",
    )
    .bind(book_id.to_string())
    .bind(source_sha256)
    .bind(format!("books/{book_id}/original.pdf"))
    .bind(timestamp)
    .bind(timestamp)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO provider_profiles (id, provider_kind, display_name, model_id, context_window_tokens, created_at, updated_at, validated_at) VALUES (?, 'openai', 'Synthetic coordinator profile', 'gpt-5.6', 1050000, ?, ?, ?)",
    )
    .bind(profile_id.to_string())
    .bind(timestamp)
    .bind(timestamp)
    .bind(timestamp)
    .execute(pool)
    .await
    .unwrap();
    for category in ["image_send", "ai_index", "cost_risk"] {
        sqlx::query(
            "INSERT INTO provider_operation_consents (profile_id, category, decision, updated_at) VALUES (?, ?, 'ask', ?)",
        )
        .bind(profile_id.to_string())
        .bind(category)
        .bind(timestamp)
        .execute(pool)
        .await
        .unwrap();
    }
}

fn submission(claim: &super::coordinator::RenderClaimDto, unique: u8) -> RenderedPageSubmission {
    let mut bytes =
        b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0dIHDR\x00\x00\x00\x01\x00\x00\x00\x01".to_vec();
    bytes.push(unique);
    RenderedPageSubmission {
        run_id: claim.run_id,
        book_id: claim.book_id,
        page_id: claim.page_id,
        page_number: claim.page_number,
        attempt_id: claim.attempt_id,
        schema_version: 1,
        mime_type: "image/png".to_owned(),
        width: 1,
        height: 1,
        decoded_pixel_count: 1,
        encoded_byte_length: u64::try_from(bytes.len()).unwrap(),
        sha256: hex(&bytes),
        bytes,
    }
}

async fn page_numbers(pool: &SqlitePool, run_id: Uuid) -> Vec<u32> {
    sqlx::query("SELECT page_number FROM index_pages WHERE run_id = ? ORDER BY page_number")
        .bind(run_id.to_string())
        .fetch_all(pool)
        .await
        .unwrap()
        .into_iter()
        .map(|row| u32::try_from(row.get::<i64, _>("page_number")).unwrap())
        .collect()
}

fn hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
