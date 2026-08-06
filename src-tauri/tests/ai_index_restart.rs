use std::{
    collections::HashMap,
    fs,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use secrecy::SecretString;
use sqlx::{AssertSqlSafe, Row, SqlitePool};
use textbooklens_lib::{
    credentials::CredentialStore,
    db::{
        Database,
        corrections::{
            CorrectionConflictDecision, DeleteIndexCorrection, ResolveIndexCorrectionConflict,
            SaveIndexCorrection, correction_value_sha256, delete_index_correction,
            list_page_corrections, resolve_index_correction_conflict, save_index_correction,
        },
        indexing::{CreateIndexRun, IndexPageOwnership, create_index_run, get_run_aggregate},
    },
    domain::{
        ContentSource, DocumentLocator, IndexCorrectionConflictState, IndexCorrectionValueKind,
        IndexFailureCode, IndexPageBlockKind, IndexPageStatus, IndexQualityReason, NormalizedRect,
        ProviderKind, RemoteCleanupHandle,
    },
    errors::{AppError, AppErrorCode, AppErrorDto, AppResult},
    indexing::{
        commit::{
            CommitBarrier, CommitControl, PageCommitRequest, commit_validated_page,
            commit_validated_page_with_control,
        },
        coordinator::IndexOperationRegistry,
        recovery::{recover_interrupted_pages, scratch_path},
        remote_cleanup::{
            RemoteResourceCleaner, recover_remote_cleanup_on_startup,
            safely_dispose_failed_resource, store_remote_handle, sweep_remote_resources,
        },
        state,
        validator::{ValidatedBlock, ValidatedPage},
    },
    retrieval::search::search_book,
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const BODY_SENTINEL: &str = "provider-body-private-sentinel-p9";
const IMAGE_SENTINEL: &str = "page-image-private-sentinel-p9";
const CREDENTIAL_SENTINEL: &str = "credential-private-sentinel-p9";
const PROMPT_SENTINEL: &str = "provider-prompt-private-sentinel-p9";
const OPAQUE_SENTINEL: &str = "opaque-remote-private-sentinel-p9";
const SOURCE_PATH_SENTINEL: &str = "C:\\private\\source-textbook-private-sentinel-p9.pdf";
const FORBIDDEN_SENTINELS: [&str; 6] = [
    BODY_SENTINEL,
    IMAGE_SENTINEL,
    CREDENTIAL_SENTINEL,
    PROMPT_SENTINEL,
    OPAQUE_SENTINEL,
    SOURCE_PATH_SENTINEL,
];

#[test]
fn restart_recovers_every_transient_page_and_keeps_committed_fts_offline() {
    let temporary = tempfile::tempdir().unwrap();
    let database_path = temporary.path().join("ai-index-restart.sqlite3");
    let scratch_root = temporary.path().join("indexing-scratch");
    let source = temporary.path().join("synthetic-source.pdf");
    let network = temporary.path().join("loopback-provider.available");
    fs::write(&source, b"self-made synthetic source").unwrap();
    fs::write(&network, b"loopback only").unwrap();

    let database = Database::open(&database_path).unwrap();
    let fixture = tauri::async_runtime::block_on(async {
        let fixture = create_fixture(database.pool(), "Restart recovery", 'a', 6).await;
        let rendering = fixture.pages[0];
        let sending = fixture.pages[1];
        let parsing = fixture.pages[2];
        let validating = fixture.pages[3];
        let indexed = fixture.pages[4];

        state::claim_render(
            database.pool(),
            rendering.page_id,
            rendering.attempt_id.unwrap(),
        )
        .await
        .unwrap();
        drive_to_sending(database.pool(), sending).await;
        drive_to_parsing(database.pool(), parsing).await;
        drive_to_validating(database.pool(), validating).await;
        drive_to_parsing(database.pool(), indexed).await;
        commit_text(
            database.pool(),
            indexed.page_id,
            indexed.attempt_id.unwrap(),
            5,
            "durable optics survives scratch source network removal",
            "safe synthetic optics diagram",
        )
        .await;

        set_updated_at(
            database.pool(),
            rendering.page_id,
            "2026-08-04T00:55:00.000Z",
        )
        .await;
        set_updated_at(database.pool(), sending.page_id, "2026-08-04T00:55:00.000Z").await;
        set_updated_at(database.pool(), parsing.page_id, "2026-08-04T00:55:00.000Z").await;
        set_updated_at(
            database.pool(),
            validating.page_id,
            "2026-08-04T00:00:00.000Z",
        )
        .await;

        let sending_scratch =
            scratch_path(&scratch_root, sending.page_id, sending.attempt_id.unwrap());
        fs::create_dir_all(sending_scratch.parent().unwrap()).unwrap();
        fs::write(&sending_scratch, b"synthetic reusable render").unwrap();
        let validating_scratch = scratch_path(
            &scratch_root,
            validating.page_id,
            validating.attempt_id.unwrap(),
        );
        fs::create_dir_all(validating_scratch.parent().unwrap()).unwrap();
        fs::write(&validating_scratch, b"synthetic expired render").unwrap();

        let operations = IndexOperationRegistry::default();
        assert_eq!(
            format!("{operations:?}"),
            "IndexOperationRegistry { pending_count: 0, authorized_run_count: 0 }"
        );
        database.pool().close().await;
        fixture
    });
    drop(database);

    let database = Database::open(&database_path).unwrap();
    tauri::async_runtime::block_on(async {
        let restarted_operations = IndexOperationRegistry::default();
        assert_eq!(
            format!("{restarted_operations:?}"),
            "IndexOperationRegistry { pending_count: 0, authorized_run_count: 0 }"
        );
        let now = DateTime::parse_from_rfc3339("2026-08-04T01:00:00.000Z")
            .unwrap()
            .with_timezone(&Utc);
        let summary = recover_interrupted_pages(
            database.pool(),
            &scratch_root,
            now,
            Duration::from_secs(10 * 60),
        )
        .await
        .unwrap();
        assert_eq!(summary.requeued, 2);
        assert_eq!(summary.marked_retryable, 2);
        assert_eq!(summary.unchanged_indexed, 1);

        let rendering = page_snapshot(database.pool(), fixture.pages[0].page_id).await;
        assert_eq!(rendering.status, IndexPageStatus::Queued);
        assert_ne!(rendering.attempt_id, fixture.pages[0].attempt_id.unwrap());
        assert_eq!(rendering.attempt_count, 2);
        assert!(!rendering.has_render_hash);

        let sending = page_snapshot(database.pool(), fixture.pages[1].page_id).await;
        assert_eq!(sending.status, IndexPageStatus::Queued);
        assert_ne!(sending.attempt_id, fixture.pages[1].attempt_id.unwrap());
        assert_eq!(sending.attempt_count, 2);
        assert!(sending.has_render_hash);
        assert!(
            scratch_path(&scratch_root, fixture.pages[1].page_id, sending.attempt_id).is_file()
        );

        let parsing = page_snapshot(database.pool(), fixture.pages[2].page_id).await;
        assert_eq!(parsing.status, IndexPageStatus::Failed);
        assert_eq!(
            parsing.safe_error_code.as_deref(),
            Some(IndexFailureCode::IndexScratchMissing.as_str())
        );
        assert!(parsing.retryable);

        let validating = page_snapshot(database.pool(), fixture.pages[3].page_id).await;
        assert_eq!(validating.status, IndexPageStatus::Failed);
        assert_eq!(
            validating.safe_error_code.as_deref(),
            Some(IndexFailureCode::IndexAttemptExpired.as_str())
        );
        assert!(validating.retryable);

        let indexed = page_snapshot(database.pool(), fixture.pages[4].page_id).await;
        assert_eq!(indexed.status, IndexPageStatus::Indexed);
        assert_eq!(indexed.content_version, 1);
        let queued = page_snapshot(database.pool(), fixture.pages[5].page_id).await;
        assert_eq!(queued.status, IndexPageStatus::Queued);
        assert_eq!(queued.attempt_id, fixture.pages[5].attempt_id.unwrap());
        assert_eq!(queued.attempt_count, 1);

        let late = state::mark_received(
            database.pool(),
            fixture.pages[1].page_id,
            fixture.pages[1].attempt_id.unwrap(),
            &"f".repeat(64),
        )
        .await
        .unwrap_err();
        assert_eq!(late.code, AppErrorCode::RequestConflict);

        let second = recover_interrupted_pages(
            database.pool(),
            &scratch_root,
            now,
            Duration::from_secs(10 * 60),
        )
        .await
        .unwrap();
        assert_eq!(second.requeued, 0);
        assert_eq!(second.marked_retryable, 0);
        assert_eq!(second.unchanged_indexed, 1);

        fs::remove_file(&source).unwrap();
        fs::remove_file(&network).unwrap();
        fs::remove_dir_all(&scratch_root).unwrap();
        let hits = search_book(database.pool(), fixture.book_id, "durable optics", 20)
            .await
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].provenance.source, ContentSource::AiTranscribed);
        assert!(hits[0].provenance.quoteable);
        assert_database_omits(database.pool(), &FORBIDDEN_SENTINELS).await;
    });
}

#[test]
fn cancellation_barriers_and_late_attempts_never_commit_cancelled_pages() {
    let temporary = tempfile::tempdir().unwrap();
    let database = Database::open(temporary.path().join("ai-index-cancel.sqlite3")).unwrap();
    tauri::async_runtime::block_on(async {
        let fixture = create_fixture(database.pool(), "Cancellation barriers", 'b', 7).await;

        let rendering = fixture.pages[0];
        state::claim_render(
            database.pool(),
            rendering.page_id,
            rendering.attempt_id.unwrap(),
        )
        .await
        .unwrap();
        state::cancel(
            database.pool(),
            rendering.page_id,
            IndexPageStatus::Rendering,
            rendering.attempt_id.unwrap(),
        )
        .await
        .unwrap();
        assert_conflict(
            state::mark_rendered(
                database.pool(),
                rendering.page_id,
                rendering.attempt_id.unwrap(),
                &"1".repeat(64),
            )
            .await,
        );

        let sending = fixture.pages[1];
        drive_to_sending(database.pool(), sending).await;
        state::cancel(
            database.pool(),
            sending.page_id,
            IndexPageStatus::Sending,
            sending.attempt_id.unwrap(),
        )
        .await
        .unwrap();
        assert_conflict(
            state::mark_received(
                database.pool(),
                sending.page_id,
                sending.attempt_id.unwrap(),
                &"2".repeat(64),
            )
            .await,
        );

        let parsing = fixture.pages[2];
        drive_to_parsing(database.pool(), parsing).await;
        state::cancel(
            database.pool(),
            parsing.page_id,
            IndexPageStatus::Parsing,
            parsing.attempt_id.unwrap(),
        )
        .await
        .unwrap();
        assert_conflict(
            state::claim_validate(
                database.pool(),
                parsing.page_id,
                parsing.attempt_id.unwrap(),
            )
            .await,
        );

        let validating = fixture.pages[3];
        drive_to_validating(database.pool(), validating).await;
        state::cancel(
            database.pool(),
            validating.page_id,
            IndexPageStatus::Validating,
            validating.attempt_id.unwrap(),
        )
        .await
        .unwrap();
        assert_conflict(
            state::commit_indexed(
                database.pool(),
                validating.page_id,
                validating.attempt_id.unwrap(),
                &"3".repeat(64),
            )
            .await,
        );

        for (index, barrier) in [
            CommitBarrier::BeforeTransaction,
            CommitBarrier::BeforeStatusAndCommit,
        ]
        .into_iter()
        .enumerate()
        {
            let page = fixture.pages[4 + index];
            drive_to_parsing(database.pool(), page).await;
            let cancellation = CancellationToken::new();
            let observed = cancellation.clone();
            let control = CommitControl::with_barrier(move |current| {
                if current == barrier {
                    observed.cancel();
                }
            });
            let validated = validated_page(
                u32::try_from(5 + index).unwrap(),
                "cancelled commit body never persists",
                "safe cancelled description",
            );
            let error = commit_validated_page_with_control(
                database.pool(),
                PageCommitRequest {
                    page_id: page.page_id,
                    attempt_id: page.attempt_id.unwrap(),
                    page: &validated,
                },
                &cancellation,
                &control,
            )
            .await
            .unwrap_err();
            assert_eq!(error.code, AppErrorCode::ImportCancelled);
            assert_eq!(
                page_snapshot(database.pool(), page.page_id).await.status,
                IndexPageStatus::Parsing
            );
            state::cancel(
                database.pool(),
                page.page_id,
                IndexPageStatus::Parsing,
                page.attempt_id.unwrap(),
            )
            .await
            .unwrap();
            assert!(
                commit_validated_page(
                    database.pool(),
                    PageCommitRequest {
                        page_id: page.page_id,
                        attempt_id: page.attempt_id.unwrap(),
                        page: &validated,
                    },
                    &CancellationToken::new(),
                )
                .await
                .is_err()
            );
        }

        let committed = fixture.pages[6];
        drive_to_parsing(database.pool(), committed).await;
        let cancellation = CancellationToken::new();
        let observed = cancellation.clone();
        let control = CommitControl::with_barrier(move |barrier| {
            if barrier == CommitBarrier::AfterCommit {
                observed.cancel();
            }
        });
        let validated = validated_page(
            7,
            "after commit cancellation has a durable boundary",
            "safe final description",
        );
        commit_validated_page_with_control(
            database.pool(),
            PageCommitRequest {
                page_id: committed.page_id,
                attempt_id: committed.attempt_id.unwrap(),
                page: &validated,
            },
            &cancellation,
            &control,
        )
        .await
        .unwrap();
        assert!(cancellation.is_cancelled());
        assert_eq!(
            page_snapshot(database.pool(), committed.page_id)
                .await
                .status,
            IndexPageStatus::Indexed
        );
        assert!(
            commit_validated_page(
                database.pool(),
                PageCommitRequest {
                    page_id: committed.page_id,
                    attempt_id: committed.attempt_id.unwrap(),
                    page: &validated,
                },
                &CancellationToken::new(),
            )
            .await
            .is_err()
        );
        assert_eq!(fts_count(database.pool(), "cancelled commit body").await, 0);
        assert_eq!(
            fts_count(database.pool(), "after commit cancellation").await,
            1
        );
    });
}

#[test]
fn correction_reanalysis_conflicts_require_explicit_cas_decisions() {
    let temporary = tempfile::tempdir().unwrap();
    let database = Database::open(temporary.path().join("ai-index-corrections.sqlite3")).unwrap();
    tauri::async_runtime::block_on(async {
        let profile_id = insert_profile(database.pool()).await;
        let book_id = insert_book(database.pool(), "Correction target", 'c').await;
        let other_book_id = insert_book(database.pool(), "Correction decoy", 'd').await;
        let mut page = create_indexed_page(
            database.pool(),
            book_id,
            profile_id,
            "provider alpha original",
            "safe alpha diagram",
        )
        .await;
        let created =
            save_text_correction(database.pool(), &page, "human overlay alpha retained", 0).await;

        reanalyze_page(database.pool(), &mut page, "provider alpha original").await;
        let unchanged = only_correction(database.pool(), page.page_id).await;
        assert_eq!(unchanged.id, created.id);
        assert_eq!(
            unchanged.conflict_state,
            IndexCorrectionConflictState::Active
        );
        let retained = search_book(database.pool(), book_id, "human overlay alpha", 20)
            .await
            .unwrap();
        assert_eq!(retained.len(), 1);
        assert_eq!(retained[0].provenance.source, ContentSource::UserCorrected);
        assert_eq!(retained[0].provenance.correction_id, Some(created.id));

        reanalyze_page(database.pool(), &mut page, "changed provider gamma value").await;
        let conflicted = only_correction(database.pool(), page.page_id).await;
        assert_eq!(
            conflicted.conflict_state,
            IndexCorrectionConflictState::Conflict
        );
        assert!(
            search_book(database.pool(), book_id, "changed provider gamma", 20)
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            search_book(database.pool(), book_id, "human overlay alpha", 20)
                .await
                .unwrap()
                .is_empty()
        );

        let kept = resolve_index_correction_conflict(
            database.pool(),
            ResolveIndexCorrectionConflict {
                book_id,
                page_id: page.page_id,
                correction_id: conflicted.id,
                target_content_version: page.content_version,
                current_value_sha256: Some(correction_value_sha256(&page.provider_value)),
                expected_revision: conflicted.revision,
                decision: CorrectionConflictDecision::Keep,
                compared_corrected_value: None,
            },
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(kept.conflict_state, IndexCorrectionConflictState::Active);

        reanalyze_page(database.pool(), &mut page, "accepted provider delta value").await;
        let accept_conflict = only_correction(database.pool(), page.page_id).await;
        let accepted = resolve_index_correction_conflict(
            database.pool(),
            ResolveIndexCorrectionConflict {
                book_id,
                page_id: page.page_id,
                correction_id: accept_conflict.id,
                target_content_version: page.content_version,
                current_value_sha256: Some(correction_value_sha256(&page.provider_value)),
                expected_revision: accept_conflict.revision,
                decision: CorrectionConflictDecision::Accept,
                compared_corrected_value: None,
            },
        )
        .await
        .unwrap();
        assert!(accepted.is_none());
        assert!(
            list_page_corrections(database.pool(), page.page_id)
                .await
                .unwrap()
                .is_empty()
        );
        let provider = search_book(database.pool(), book_id, "accepted provider delta", 20)
            .await
            .unwrap();
        assert_eq!(provider.len(), 1);
        assert_eq!(provider[0].provenance.source, ContentSource::AiTranscribed);

        save_text_correction(database.pool(), &page, "temporary human comparison", 0).await;
        reanalyze_page(
            database.pool(),
            &mut page,
            "compared provider epsilon value",
        )
        .await;
        let compare_conflict = only_correction(database.pool(), page.page_id).await;
        let compared = resolve_index_correction_conflict(
            database.pool(),
            ResolveIndexCorrectionConflict {
                book_id,
                page_id: page.page_id,
                correction_id: compare_conflict.id,
                target_content_version: page.content_version,
                current_value_sha256: Some(correction_value_sha256(&page.provider_value)),
                expected_revision: compare_conflict.revision,
                decision: CorrectionConflictDecision::Compare,
                compared_corrected_value: Some("comparison saved human epsilon".to_owned()),
            },
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(compared.original_value, "compared provider epsilon value");
        assert_eq!(compared.corrected_value, "comparison saved human epsilon");

        let cross_book = save_index_correction(
            database.pool(),
            SaveIndexCorrection {
                book_id: other_book_id,
                page_id: page.page_id,
                target_block_id: page.block_id,
                target_content_version: page.content_version,
                value_kind: IndexCorrectionValueKind::Text,
                original_value_sha256: correction_value_sha256(&page.provider_value),
                corrected_value: "cross-book rejected".to_owned(),
                expected_revision: compared.revision,
            },
        )
        .await
        .unwrap_err();
        assert_eq!(cross_book.code, AppErrorCode::RequestConflict);

        let stale_delete = delete_index_correction(
            database.pool(),
            DeleteIndexCorrection {
                book_id,
                page_id: page.page_id,
                correction_id: compared.id,
                target_content_version: page.content_version,
                current_value_sha256: Some(correction_value_sha256(&page.provider_value)),
                expected_revision: compared.revision + 1,
            },
        )
        .await
        .unwrap_err();
        assert_eq!(stale_delete.code, AppErrorCode::RequestConflict);
        delete_index_correction(
            database.pool(),
            DeleteIndexCorrection {
                book_id,
                page_id: page.page_id,
                correction_id: compared.id,
                target_content_version: page.content_version,
                current_value_sha256: Some(correction_value_sha256(&page.provider_value)),
                expected_revision: compared.revision,
            },
        )
        .await
        .unwrap();
        assert!(
            search_book(
                database.pool(),
                book_id,
                "comparison saved human epsilon",
                20
            )
            .await
            .unwrap()
            .is_empty()
        );
        let revealed = search_book(database.pool(), book_id, "compared provider epsilon", 20)
            .await
            .unwrap();
        assert_eq!(revealed.len(), 1);
        assert_eq!(revealed[0].provenance.source, ContentSource::AiTranscribed);
    });
}

#[test]
fn identical_wording_remains_source_distinct_weighted_quoteable_and_book_isolated() {
    let temporary = tempfile::tempdir().unwrap();
    let database = Database::open(temporary.path().join("ai-index-provenance.sqlite3")).unwrap();
    tauri::async_runtime::block_on(async {
        let profile_id = insert_profile(database.pool()).await;
        let book_id = insert_book(database.pool(), "Provenance target", 'e').await;
        let decoy_id = insert_book(database.pool(), "Provenance decoy", 'f').await;
        let phrase = "shared provenance phrase";
        insert_local_chunk(database.pool(), book_id, phrase).await;
        insert_local_chunk(
            database.pool(),
            decoy_id,
            &format!(
                "{} decoy-only provenance marker",
                format!("{phrase} ").repeat(80)
            ),
        )
        .await;
        create_indexed_page(database.pool(), book_id, profile_id, phrase, phrase).await;
        let corrected_page = create_indexed_page(
            database.pool(),
            book_id,
            profile_id,
            phrase,
            "unrelated safe diagram",
        )
        .await;
        let correction = save_text_correction(database.pool(), &corrected_page, phrase, 0).await;

        let hits = search_book(database.pool(), book_id, phrase, 20)
            .await
            .unwrap();
        assert_eq!(hits.len(), 4, "equal wording must not be source-collapsed");
        for source in [
            ContentSource::LocalText,
            ContentSource::AiTranscribed,
            ContentSource::AiDescription,
            ContentSource::UserCorrected,
        ] {
            assert_eq!(
                hits.iter()
                    .filter(|hit| hit.provenance.source == source)
                    .count(),
                1
            );
        }
        let description = hits
            .iter()
            .find(|hit| hit.provenance.source == ContentSource::AiDescription)
            .unwrap();
        let transcription = hits
            .iter()
            .find(|hit| hit.provenance.source == ContentSource::AiTranscribed)
            .unwrap();
        assert!(!description.provenance.quoteable);
        assert!(transcription.provenance.quoteable);
        assert!(
            description.provenance.ranking_weight() < transcription.provenance.ranking_weight()
        );
        let corrected = hits
            .iter()
            .find(|hit| hit.provenance.source == ContentSource::UserCorrected)
            .unwrap();
        assert!(corrected.provenance.quoteable);
        assert_eq!(corrected.provenance.correction_id, Some(correction.id));
        assert_eq!(
            corrected.provenance.original_source,
            Some(ContentSource::AiTranscribed)
        );
        assert_eq!(
            corrected.provenance.original_value_sha256,
            Some(correction_value_sha256(phrase))
        );
        assert!(
            hits.iter()
                .all(|hit| !hit.snippet.contains("decoy-only provenance marker"))
        );
    });
}

#[test]
fn remote_cleanup_retries_after_restart_is_at_most_once_and_never_leaks_secrets() {
    let temporary = tempfile::tempdir().unwrap();
    let database_path = temporary.path().join("ai-index-remote-cleanup.sqlite3");
    let vault = Arc::new(TestVault::default());
    let cleaner = Arc::new(FakeCleaner::default());
    cleaner.failures_remaining.store(1, Ordering::SeqCst);

    let database = Database::open(&database_path).unwrap();
    let retry_resource = tauri::async_runtime::block_on(async {
        let fixture = create_fixture(database.pool(), "Remote cleanup", '1', 1).await;
        let handle = RemoteCleanupHandle::new(
            ProviderKind::OpenAi,
            SecretString::from(OPAQUE_SENTINEL.to_owned()),
        );
        assert_surface_omits(&format!("{handle:?}"), &FORBIDDEN_SENTINELS);
        let resource_id = store_remote_handle(
            database.pool(),
            vault.as_ref(),
            fixture.pages[0].page_id,
            &handle,
        )
        .await
        .unwrap();
        let first = sweep_remote_resources(
            database.pool(),
            vault.as_ref(),
            cleaner.as_ref(),
            CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(first.failed, 1);
        assert_eq!(cleanup_status(database.pool(), resource_id).await, "failed");
        assert_eq!(cleaner.calls.load(Ordering::SeqCst), 1);
        assert_database_omits(database.pool(), &FORBIDDEN_SENTINELS).await;
        database.pool().close().await;
        resource_id
    });
    drop(database);

    let database = Database::open(&database_path).unwrap();
    tauri::async_runtime::block_on(async {
        let startup = recover_remote_cleanup_on_startup(database.pool(), vault.as_ref())
            .await
            .unwrap();
        assert_eq!(startup, Default::default());
        assert_eq!(
            cleanup_status(database.pool(), retry_resource).await,
            "failed"
        );
        assert_eq!(cleaner.calls.load(Ordering::SeqCst), 1);

        let explicit_retry = sweep_remote_resources(
            database.pool(),
            vault.as_ref(),
            cleaner.as_ref(),
            CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(explicit_retry.succeeded, 1);
        assert_eq!(
            cleanup_status(database.pool(), retry_resource).await,
            "succeeded"
        );
        assert_eq!(cleaner.calls.load(Ordering::SeqCst), 2);

        let page_id = sqlx::query_scalar::<_, String>(
            "SELECT id FROM index_pages ORDER BY page_number LIMIT 1",
        )
        .fetch_one(database.pool())
        .await
        .unwrap();
        let page_id = Uuid::parse_str(&page_id).unwrap();
        let concurrent_resource = store_remote_handle(
            database.pool(),
            vault.as_ref(),
            page_id,
            &RemoteCleanupHandle::new(
                ProviderKind::OpenAi,
                SecretString::from("opaque-concurrent-private-sentinel-p9".to_owned()),
            ),
        )
        .await
        .unwrap();
        let first = sweep_remote_resources(
            database.pool(),
            vault.as_ref(),
            cleaner.as_ref(),
            CancellationToken::new(),
        );
        let second = sweep_remote_resources(
            database.pool(),
            vault.as_ref(),
            cleaner.as_ref(),
            CancellationToken::new(),
        );
        let (first, second) = tokio::join!(first, second);
        first.unwrap();
        second.unwrap();
        assert_eq!(
            cleanup_status(database.pool(), concurrent_resource).await,
            "succeeded"
        );
        assert_eq!(cleaner.calls.load(Ordering::SeqCst), 3);

        let missing_resource = store_remote_handle(
            database.pool(),
            vault.as_ref(),
            page_id,
            &RemoteCleanupHandle::new(
                ProviderKind::OpenAi,
                SecretString::from("opaque-missing-private-sentinel-p9".to_owned()),
            ),
        )
        .await
        .unwrap();
        let missing_key = vault.key_for_only_opaque("opaque-missing-private-sentinel-p9");
        vault.delete(&missing_key).await.unwrap();
        let missing = sweep_remote_resources(
            database.pool(),
            vault.as_ref(),
            cleaner.as_ref(),
            CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(missing.failed, 1);
        assert_eq!(cleaner.calls.load(Ordering::SeqCst), 3);
        let missing_row = sqlx::query(
            "SELECT cleanup_status, cleanup_attempt_id, safe_error_code FROM provider_remote_resources WHERE id = ?",
        )
        .bind(missing_resource.to_string())
        .fetch_one(database.pool())
        .await
        .unwrap();
        assert_eq!(missing_row.get::<String, _>("cleanup_status"), "failed");
        assert_eq!(
            missing_row.get::<String, _>("safe_error_code"),
            "REMOTE_AUTH_REQUIRED"
        );
        let missing_attempt =
            Uuid::parse_str(&missing_row.get::<String, _>("cleanup_attempt_id")).unwrap();
        let denied = safely_dispose_failed_resource(
            database.pool(),
            vault.as_ref(),
            missing_resource,
            missing_attempt,
            false,
        )
        .await
        .unwrap_err();
        assert_eq!(denied.code, AppErrorCode::InvalidInput);
        safely_dispose_failed_resource(
            database.pool(),
            vault.as_ref(),
            missing_resource,
            missing_attempt,
            true,
        )
        .await
        .unwrap();
        assert_eq!(
            cleanup_status(database.pool(), missing_resource).await,
            "safely_disposed"
        );

        let marker_resource = store_remote_handle(
            database.pool(),
            vault.as_ref(),
            page_id,
            &RemoteCleanupHandle::new(
                ProviderKind::OpenAi,
                SecretString::from("opaque-marker-private-sentinel-p9".to_owned()),
            ),
        )
        .await
        .unwrap();
        let marker_key = vault.key_for_only_opaque("opaque-marker-private-sentinel-p9");
        sqlx::query(
            "UPDATE provider_remote_resources SET cleanup_status = 'cleaning', cleanup_attempt_id = ?, cleanup_attempt_count = cleanup_attempt_count + 1, last_cleanup_at = '2026-08-04T00:00:01.000Z', updated_at = '2026-08-04T00:00:01.000Z' WHERE id = ? AND cleanup_status = 'pending'",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(marker_resource.to_string())
        .execute(database.pool())
        .await
        .unwrap();
        vault
            .set(
                &marker_key,
                SecretString::from("textbooklens:remote-resource-deleted:v1".to_owned()),
            )
            .await
            .unwrap();
        let recovered = recover_remote_cleanup_on_startup(database.pool(), vault.as_ref())
            .await
            .unwrap();
        assert_eq!(recovered.recovered_success_markers, 1);
        assert_eq!(cleaner.calls.load(Ordering::SeqCst), 3);
        assert_eq!(
            cleanup_status(database.pool(), marker_resource).await,
            "succeeded"
        );
        assert_eq!(
            recover_remote_cleanup_on_startup(database.pool(), vault.as_ref())
                .await
                .unwrap(),
            Default::default()
        );

        let error = AppError::invalid_api_key(CREDENTIAL_SENTINEL, BODY_SENTINEL);
        assert_surface_omits(&format!("{error:?}"), &FORBIDDEN_SENTINELS);
        assert_surface_omits(&error.to_string(), &FORBIDDEN_SENTINELS);
        let dto = AppErrorDto::from(error);
        assert_surface_omits(&serde_json::to_string(&dto).unwrap(), &FORBIDDEN_SENTINELS);
        let aggregate = get_run_aggregate(
            database.pool(),
            Uuid::parse_str(
                &sqlx::query_scalar::<_, String>("SELECT id FROM index_runs LIMIT 1")
                    .fetch_one(database.pool())
                    .await
                    .unwrap(),
            )
            .unwrap(),
        )
        .await
        .unwrap();
        assert_surface_omits(
            &serde_json::to_string(&aggregate).unwrap(),
            &FORBIDDEN_SENTINELS,
        );
        assert_database_omits(database.pool(), &FORBIDDEN_SENTINELS).await;
        assert_eq!(vault.count_opaque_values(), 0);
    });
}

struct Fixture {
    book_id: Uuid,
    pages: Vec<IndexPageOwnership>,
}

async fn create_fixture(
    pool: &SqlitePool,
    title: &str,
    hash_character: char,
    page_count: u32,
) -> Fixture {
    let profile_id = insert_profile(pool).await;
    let book_id = insert_book(pool, title, hash_character).await;
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
    let mut pages = Vec::new();
    for page_number in 1..=page_count {
        pages.push(
            state::queue(pool, run_id, page_number, IndexQualityReason::NoText, None)
                .await
                .unwrap(),
        );
    }
    Fixture { book_id, pages }
}

async fn insert_profile(pool: &SqlitePool) -> Uuid {
    let profile_id = Uuid::new_v4();
    let timestamp = "2026-08-04T00:00:00.000Z";
    sqlx::query(
        "INSERT INTO provider_profiles (id, provider_kind, display_name, model_id, context_window_tokens, created_at, updated_at, validated_at) VALUES (?, 'openai', 'Synthetic checkpoint profile', 'synthetic-vision-v1', 32000, ?, ?, ?)",
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

async fn insert_book(pool: &SqlitePool, title: &str, hash_character: char) -> Uuid {
    let book_id = Uuid::new_v4();
    let timestamp = "2026-08-04T00:00:00.000Z";
    sqlx::query(
        "INSERT INTO books (id, sha256, title, format, original_filename, stored_path, import_status, created_at, updated_at) VALUES (?, ?, ?, 'pdf', 'synthetic.pdf', ?, 'ready', ?, ?)",
    )
    .bind(book_id.to_string())
    .bind(hash_character.to_string().repeat(64))
    .bind(title)
    .bind(format!("books/{book_id}/original.pdf"))
    .bind(timestamp)
    .bind(timestamp)
    .execute(pool)
    .await
    .unwrap();
    book_id
}

async fn drive_to_sending(pool: &SqlitePool, page: IndexPageOwnership) {
    let attempt_id = page.attempt_id.unwrap();
    state::claim_render(pool, page.page_id, attempt_id)
        .await
        .unwrap();
    state::mark_rendered(pool, page.page_id, attempt_id, &"a".repeat(64))
        .await
        .unwrap();
    state::claim_send(pool, page.page_id, attempt_id)
        .await
        .unwrap();
}

async fn drive_to_parsing(pool: &SqlitePool, page: IndexPageOwnership) {
    drive_to_sending(pool, page).await;
    state::mark_received(
        pool,
        page.page_id,
        page.attempt_id.unwrap(),
        &"b".repeat(64),
    )
    .await
    .unwrap();
}

async fn drive_to_validating(pool: &SqlitePool, page: IndexPageOwnership) {
    drive_to_parsing(pool, page).await;
    state::claim_validate(pool, page.page_id, page.attempt_id.unwrap())
        .await
        .unwrap();
}

fn validated_page(page_number: u32, text: &str, description: &str) -> ValidatedPage {
    ValidatedPage {
        page_number,
        review_reason: None,
        blocks: vec![ValidatedBlock {
            ordinal: 0,
            kind: IndexPageBlockKind::Paragraph,
            plain_text: Some(text.to_owned()),
            latex: Some("E = K + U".to_owned()),
            table_cells: None,
            visual_description: Some(description.to_owned()),
            bounds: Some(NormalizedRect::new(0.1, 0.1, 0.8, 0.2).unwrap()),
            source: ContentSource::AiTranscribed,
        }],
    }
}

async fn commit_text(
    pool: &SqlitePool,
    page_id: Uuid,
    attempt_id: Uuid,
    page_number: u32,
    text: &str,
    description: &str,
) {
    let page = validated_page(page_number, text, description);
    commit_validated_page(
        pool,
        PageCommitRequest {
            page_id,
            attempt_id,
            page: &page,
        },
        &CancellationToken::new(),
    )
    .await
    .unwrap();
}

struct IndexedPage {
    book_id: Uuid,
    page_id: Uuid,
    attempt_id: Uuid,
    block_id: Uuid,
    content_version: u32,
    provider_value: String,
}

async fn create_indexed_page(
    pool: &SqlitePool,
    book_id: Uuid,
    profile_id: Uuid,
    provider_value: &str,
    description: &str,
) -> IndexedPage {
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
    drive_to_parsing(pool, page).await;
    commit_text(
        pool,
        page.page_id,
        attempt_id,
        1,
        provider_value,
        description,
    )
    .await;
    let block_id: String = sqlx::query_scalar(
        "SELECT id FROM index_page_blocks WHERE page_id = ? AND content_version = 1 AND ordinal = 0",
    )
    .bind(page.page_id.to_string())
    .fetch_one(pool)
    .await
    .unwrap();
    IndexedPage {
        book_id,
        page_id: page.page_id,
        attempt_id,
        block_id: Uuid::parse_str(&block_id).unwrap(),
        content_version: 1,
        provider_value: provider_value.to_owned(),
    }
}

async fn reanalyze_page(pool: &SqlitePool, page: &mut IndexedPage, provider_value: &str) {
    let attempt_id = state::retry(
        pool,
        page.page_id,
        IndexPageStatus::Indexed,
        page.attempt_id,
    )
    .await
    .unwrap();
    let ownership = IndexPageOwnership {
        page_id: page.page_id,
        run_id: Uuid::nil(),
        book_id: page.book_id,
        attempt_id: Some(attempt_id),
        status: IndexPageStatus::Queued,
    };
    drive_to_parsing(pool, ownership).await;
    commit_text(
        pool,
        page.page_id,
        attempt_id,
        1,
        provider_value,
        "safe reanalysis diagram",
    )
    .await;
    page.attempt_id = attempt_id;
    page.content_version += 1;
    page.provider_value = provider_value.to_owned();
}

async fn save_text_correction(
    pool: &SqlitePool,
    page: &IndexedPage,
    corrected_value: &str,
    expected_revision: u32,
) -> textbooklens_lib::domain::IndexCorrectionReviewDto {
    save_index_correction(
        pool,
        SaveIndexCorrection {
            book_id: page.book_id,
            page_id: page.page_id,
            target_block_id: page.block_id,
            target_content_version: page.content_version,
            value_kind: IndexCorrectionValueKind::Text,
            original_value_sha256: correction_value_sha256(&page.provider_value),
            corrected_value: corrected_value.to_owned(),
            expected_revision,
        },
    )
    .await
    .unwrap()
}

async fn only_correction(
    pool: &SqlitePool,
    page_id: Uuid,
) -> textbooklens_lib::domain::IndexCorrectionReviewDto {
    let corrections = list_page_corrections(pool, page_id).await.unwrap();
    assert_eq!(corrections.len(), 1);
    corrections.into_iter().next().unwrap()
}

async fn insert_local_chunk(pool: &SqlitePool, book_id: Uuid, text: &str) {
    let section_id = Uuid::new_v4();
    let locator = serde_json::to_string(&DocumentLocator::pdf(1, 1, None).unwrap()).unwrap();
    sqlx::query(
        "INSERT INTO sections (id, book_id, ordinal, title, locator_json) VALUES (?, ?, 0, 'Synthetic local source', ?)",
    )
    .bind(section_id.to_string())
    .bind(book_id.to_string())
    .bind(&locator)
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
    .bind(locator)
    .execute(pool)
    .await
    .unwrap();
}

#[derive(Debug)]
struct PageSnapshot {
    status: IndexPageStatus,
    attempt_id: Uuid,
    attempt_count: u32,
    has_render_hash: bool,
    safe_error_code: Option<String>,
    retryable: bool,
    content_version: u32,
}

async fn page_snapshot(pool: &SqlitePool, page_id: Uuid) -> PageSnapshot {
    let row = sqlx::query(
        "SELECT status, attempt_id, attempt_count, render_sha256, safe_error_code, retryable, content_version FROM index_pages WHERE id = ?",
    )
    .bind(page_id.to_string())
    .fetch_one(pool)
    .await
    .unwrap();
    PageSnapshot {
        status: IndexPageStatus::from_database(&row.get::<String, _>("status")).unwrap(),
        attempt_id: Uuid::parse_str(&row.get::<String, _>("attempt_id")).unwrap(),
        attempt_count: u32::try_from(row.get::<i64, _>("attempt_count")).unwrap(),
        has_render_hash: row.get::<Option<String>, _>("render_sha256").is_some(),
        safe_error_code: row.get("safe_error_code"),
        retryable: row.get::<i64, _>("retryable") == 1,
        content_version: u32::try_from(row.get::<i64, _>("content_version")).unwrap(),
    }
}

async fn set_updated_at(pool: &SqlitePool, page_id: Uuid, timestamp: &str) {
    sqlx::query("UPDATE index_pages SET updated_at = ? WHERE id = ?")
        .bind(timestamp)
        .bind(page_id.to_string())
        .execute(pool)
        .await
        .unwrap();
}

fn assert_conflict(result: AppResult<()>) {
    assert_eq!(result.unwrap_err().code, AppErrorCode::RequestConflict);
}

async fn fts_count(pool: &SqlitePool, phrase: &str) -> i64 {
    let query = format!("\"{}\"", phrase.replace('"', "\"\""));
    sqlx::query_scalar(
        "SELECT COUNT(*) FROM index_search_chunks_fts WHERE index_search_chunks_fts MATCH ?",
    )
    .bind(query)
    .fetch_one(pool)
    .await
    .unwrap()
}

#[derive(Default)]
struct TestVault {
    values: Mutex<HashMap<String, SecretString>>,
}

impl TestVault {
    fn key_for_only_opaque(&self, value: &str) -> String {
        use secrecy::ExposeSecret;

        self.values
            .lock()
            .unwrap()
            .iter()
            .find(|(_, candidate)| candidate.expose_secret() == value)
            .map(|(key, _)| key.clone())
            .unwrap()
    }

    fn count_opaque_values(&self) -> usize {
        use secrecy::ExposeSecret;

        self.values
            .lock()
            .unwrap()
            .values()
            .filter(|value| value.expose_secret().contains("opaque-"))
            .count()
    }
}

#[async_trait]
impl CredentialStore for TestVault {
    async fn set(&self, key: &str, value: SecretString) -> AppResult<()> {
        self.values.lock().unwrap().insert(key.to_owned(), value);
        Ok(())
    }

    async fn get(&self, key: &str) -> AppResult<SecretString> {
        self.values
            .lock()
            .unwrap()
            .get(key)
            .cloned()
            .ok_or_else(|| AppError::credential_store("synthetic missing vault entry"))
    }

    async fn delete(&self, key: &str) -> AppResult<()> {
        self.values.lock().unwrap().remove(key);
        Ok(())
    }
}

#[derive(Default)]
struct FakeCleaner {
    calls: AtomicUsize,
    failures_remaining: AtomicUsize,
}

#[async_trait]
impl RemoteResourceCleaner for FakeCleaner {
    async fn delete(
        &self,
        _pool: &SqlitePool,
        _profile_id: Uuid,
        handle: &RemoteCleanupHandle,
        _cancel: CancellationToken,
    ) -> AppResult<()> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        assert_eq!(handle.provider(), &ProviderKind::OpenAi);
        if self.failures_remaining.load(Ordering::SeqCst) > 0 {
            self.failures_remaining.fetch_sub(1, Ordering::SeqCst);
            return Err(AppError::new(AppErrorCode::ProviderUnavailable));
        }
        Ok(())
    }
}

async fn cleanup_status(pool: &SqlitePool, resource_id: Uuid) -> String {
    sqlx::query_scalar("SELECT cleanup_status FROM provider_remote_resources WHERE id = ?")
        .bind(resource_id.to_string())
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn assert_database_omits(pool: &SqlitePool, sentinels: &[&str]) {
    let tables = sqlx::query_scalar::<_, String>(
        "SELECT name FROM sqlite_schema WHERE type = 'table' ORDER BY name",
    )
    .fetch_all(pool)
    .await
    .unwrap();
    let encoded = sentinels
        .iter()
        .map(|sentinel| hex(sentinel.as_bytes()))
        .collect::<Vec<_>>();
    for table in tables {
        let quoted_table = quote_identifier(&table);
        let columns = sqlx::query(AssertSqlSafe(format!("PRAGMA table_info({quoted_table})")))
            .fetch_all(pool)
            .await
            .unwrap();
        for column in columns {
            let name: String = column.get("name");
            let quoted_column = quote_identifier(&name);
            let values = sqlx::query_scalar::<_, String>(AssertSqlSafe(format!(
                "SELECT hex(CAST({quoted_column} AS BLOB)) FROM {quoted_table} WHERE {quoted_column} IS NOT NULL"
            )))
            .fetch_all(pool)
            .await
            .unwrap();
            for value in values {
                for sentinel in &encoded {
                    assert!(
                        !value.contains(sentinel),
                        "sensitive sentinel leaked into SQLite table {table} column {name}"
                    );
                }
            }
        }
    }
}

fn assert_surface_omits(surface: &str, sentinels: &[&str]) {
    for sentinel in sentinels {
        assert!(!surface.contains(sentinel));
    }
}

fn quote_identifier(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}
