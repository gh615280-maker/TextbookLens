use std::{
    fs,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use secrecy::{ExposeSecret, SecretString};
use sha2::{Digest, Sha256};
use sqlx::{Row, SqlitePool};
use tempfile::TempDir;
use textbooklens_lib::{
    app_state::AppPaths,
    credentials::{CredentialStore, MemoryCredentialStore},
    db::{
        Database,
        conversations::load_selection_conversation,
        corrections::{SaveIndexCorrection, correction_value_sha256, save_index_correction},
        indexing::{CreateIndexRun, IndexPageOwnership, create_index_run},
    },
    documents::import::ImportCancellationRegistry,
    domain::{
        ActiveOperationKind, ContentAnchor, ContentSource, DocumentLocator,
        IndexCorrectionValueKind, IndexPageBlockKind, IndexQualityReason, LearningRequestStatus,
        NormalizedRect, ProviderKind, RemoteCleanupHandle, SelectionAnchor, TextQuote,
    },
    errors::{AppError, AppErrorCode, AppErrorDto, AppResult},
    indexing::{
        commit::{PageCommitRequest, commit_validated_page},
        coordinator::IndexOperationRegistry,
        recovery::{recover_interrupted_pages, scratch_path},
        remote_cleanup::{
            RemoteResourceCleaner, recover_remote_cleanup_on_startup, store_remote_handle,
            sweep_remote_resources,
        },
        state,
        validator::{ValidatedBlock, ValidatedPage},
    },
    learning::registry::{
        LearningPersistenceTarget, LearningRequestContext, LearningRequestRegistry,
        NewBookQuestionRequestContext,
    },
    maintenance::{
        archive::{BackupBoundary, BackupFaultInjector, BackupInjectedFailure, BackupService},
        clear_all::{
            CLEAR_ALL_DATA_CONFIRMATION_PHRASE, ClearAllDataService, ClearBoundary,
            ClearFaultInjector, ClearInjectedCrash, ClearStartupOutcome, recover_pending_clear,
        },
        delete_book::{
            DeleteBookService, DeleteBoundary, DeleteFaultInjector, DeleteInjectedCrash,
            recover_pending_deletions_async,
        },
        gate::{GateAcquireError, MaintenanceGate},
        restore::{
            RestoreBoundary, RestoreFaultInjector, RestoreInjectedCrash, RestoreService,
            recover_pending_restore, recover_pending_restore_with_fault,
        },
        storage::{APP_DATA_DIRECTORY_NAME, prepare_app_data_paths},
    },
    retrieval::search::search_book,
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const TIME: &str = "2026-08-07T00:00:00.000Z";
const PARTIAL_STREAM: &str = "P15_PARTIAL_STREAM_MUST_NOT_SURVIVE";
const OPAQUE_REMOTE: &str = "P15_OPAQUE_REMOTE_HANDLE";
const PRIVATE_KEY: &str = "P15_PRIVATE_CREDENTIAL_VALUE";

#[test]
fn startup_recovery_is_local_idempotent_and_preserves_only_committed_state() {
    let fixture = Fixture::new();
    let scratch_root = fixture.paths.indexing_scratch();
    let credentials = Arc::new(MemoryCredentialStore::new());
    let cleaner = CountingCleaner::default();
    let database = Database::open(&fixture.paths.database).unwrap();

    let seeded = tauri::async_runtime::block_on(async {
        let import_ids = seed_interrupted_imports(database.pool(), &fixture.paths).await;
        let durable = seed_durable_book(database.pool(), &fixture.paths).await;
        let pages = seed_page_pipeline(database.pool(), durable.book_id, durable.profile_id).await;

        state::claim_render(
            database.pool(),
            pages[0].page_id,
            pages[0].attempt_id.unwrap(),
        )
        .await
        .unwrap();
        drive_to_sending(database.pool(), pages[1]).await;
        drive_to_parsing(database.pool(), pages[2]).await;
        drive_to_validating(database.pool(), pages[3]).await;
        drive_to_parsing(database.pool(), pages[4]).await;
        commit_page(database.pool(), pages[4], 5, "durable committed page").await;

        for page in &pages[..2] {
            set_page_time(database.pool(), page.page_id, "2026-08-07T00:55:00.000Z").await;
        }
        for page in &pages[2..4] {
            set_page_time(database.pool(), page.page_id, "2026-08-07T00:00:00.000Z").await;
        }
        let sending_scratch = scratch_path(
            &scratch_root,
            pages[1].page_id,
            pages[1].attempt_id.unwrap(),
        );
        fs::create_dir_all(sending_scratch.parent().unwrap()).unwrap();
        fs::write(&sending_scratch, b"synthetic render retained for retry").unwrap();
        let validating_scratch = scratch_path(
            &scratch_root,
            pages[3].page_id,
            pages[3].attempt_id.unwrap(),
        );
        fs::create_dir_all(validating_scratch.parent().unwrap()).unwrap();
        fs::write(validating_scratch, b"expired synthetic render").unwrap();

        let block_id = Uuid::parse_str(
            &sqlx::query_scalar::<_, String>(
                "SELECT id FROM index_page_blocks WHERE page_id = ? AND ordinal = 0",
            )
            .bind(pages[4].page_id.to_string())
            .fetch_one(database.pool())
            .await
            .unwrap(),
        )
        .unwrap();
        save_index_correction(
            database.pool(),
            SaveIndexCorrection {
                book_id: durable.book_id,
                page_id: pages[4].page_id,
                target_block_id: block_id,
                target_content_version: 1,
                value_kind: IndexCorrectionValueKind::Text,
                original_value_sha256: correction_value_sha256("durable committed page"),
                corrected_value: "durable corrected page".to_owned(),
                expected_revision: 0,
            },
        )
        .await
        .unwrap();

        let pending_remote = store_remote_handle(
            database.pool(),
            credentials.as_ref(),
            pages[4].page_id,
            &RemoteCleanupHandle::new(
                ProviderKind::OpenAi,
                SecretString::from(OPAQUE_REMOTE.to_owned()),
            ),
        )
        .await
        .unwrap();
        let marker_remote = store_remote_handle(
            database.pool(),
            credentials.as_ref(),
            pages[4].page_id,
            &RemoteCleanupHandle::new(
                ProviderKind::OpenAi,
                SecretString::from("P15_ALREADY_DELETED_REMOTE".to_owned()),
            ),
        )
        .await
        .unwrap();
        install_deleted_marker(database.pool(), credentials.as_ref(), marker_remote).await;

        credentials
            .set(
                "textbooklens/provider/private-p15",
                SecretString::from(PRIVATE_KEY.to_owned()),
            )
            .await
            .unwrap();

        let registry = LearningRequestRegistry::default();
        let partial = registry
            .create(Arc::new(LearningRequestContext {
                target: LearningPersistenceTarget::NewBookQuestion(NewBookQuestionRequestContext {
                    book_id: durable.book_id,
                    question: "unfinished local question".to_owned(),
                }),
                provider_profile_id: durable.profile_id,
                model_id: "synthetic-model".to_owned(),
                available_citations: Vec::new(),
            }))
            .unwrap();
        registry
            .append_text(partial.request_id, PARTIAL_STREAM.to_owned())
            .unwrap();
        assert_eq!(
            registry.snapshot(partial.request_id).unwrap().status,
            LearningRequestStatus::Streaming
        );
        drop(registry);

        database.pool().close().await;
        Seeded {
            import_ids,
            durable,
            pages,
            partial_request: partial.request_id,
            pending_remote,
            marker_remote,
        }
    });
    drop(database);

    let reopened = Database::open(&fixture.paths.database).unwrap();
    let now = DateTime::parse_from_rfc3339("2026-08-07T01:00:00.000Z")
        .unwrap()
        .with_timezone(&Utc);
    textbooklens_lib::db::settings::recover_interrupted_imports(reopened.pool(), &fixture.paths)
        .unwrap();
    tauri::async_runtime::block_on(async {
        let index = recover_interrupted_pages(
            reopened.pool(),
            &scratch_root,
            now,
            Duration::from_secs(10 * 60),
        )
        .await
        .unwrap();
        assert_eq!(
            (
                index.requeued,
                index.marked_retryable,
                index.unchanged_indexed
            ),
            (2, 2, 1)
        );
        let remote = recover_remote_cleanup_on_startup(reopened.pool(), credentials.as_ref())
            .await
            .unwrap();
        assert_eq!(remote.recovered_success_markers, 1);
        assert_eq!(cleaner.calls.load(Ordering::SeqCst), 0);
        reopened.pool().close().await;
    });
    drop(reopened);

    let reopened = Database::open(&fixture.paths.database).unwrap();
    textbooklens_lib::db::settings::recover_interrupted_imports(reopened.pool(), &fixture.paths)
        .unwrap();
    tauri::async_runtime::block_on(async {
        let second = recover_interrupted_pages(
            reopened.pool(),
            &scratch_root,
            now,
            Duration::from_secs(10 * 60),
        )
        .await
        .unwrap();
        assert_eq!(
            (
                second.requeued,
                second.marked_retryable,
                second.unchanged_indexed
            ),
            (0, 0, 1)
        );
        assert_eq!(
            recover_remote_cleanup_on_startup(reopened.pool(), credentials.as_ref())
                .await
                .unwrap()
                .recovered_success_markers,
            0
        );
        assert_eq!(cleaner.calls.load(Ordering::SeqCst), 0);

        assert_import_recovery(reopened.pool(), &fixture.paths, &seeded.import_ids).await;
        assert_page_recovery(reopened.pool(), &seeded.pages).await;
        assert_committed_state(reopened.pool(), &seeded.durable, seeded.pages[4]).await;
        assert_eq!(
            LearningRequestRegistry::default()
                .snapshot(seeded.partial_request)
                .unwrap_err()
                .code,
            AppErrorCode::NotFound
        );
        assert_eq!(
            format!("{:?}", IndexOperationRegistry::default()),
            "IndexOperationRegistry { pending_count: 0, authorized_run_count: 0 }"
        );
        assert_eq!(
            remote_status(reopened.pool(), seeded.pending_remote).await,
            "pending"
        );
        assert_eq!(
            remote_status(reopened.pool(), seeded.marker_remote).await,
            "succeeded"
        );
        assert_eq!(
            credentials
                .get("textbooklens/provider/private-p15")
                .await
                .unwrap()
                .expose_secret(),
            PRIVATE_KEY
        );

        let explicit = sweep_remote_resources(
            reopened.pool(),
            credentials.as_ref(),
            &cleaner,
            CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(
            (explicit.claimed, explicit.succeeded, explicit.failed),
            (1, 1, 0)
        );
        assert_eq!(cleaner.calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            remote_status(reopened.pool(), seeded.pending_remote).await,
            "succeeded"
        );

        let offline = AppErrorDto::from(AppError::new(AppErrorCode::NetworkOffline));
        let surface = format!("{offline:?} {}", serde_json::to_string(&offline).unwrap());
        for private in [PRIVATE_KEY, OPAQUE_REMOTE, PARTIAL_STREAM] {
            assert!(!surface.contains(private));
        }
    });
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn queued_maintenance_is_fair_and_terminal_removal_keeps_continuation_leased() {
    let gate = MaintenanceGate::default();
    let registry = ImportCancellationRegistry::default();
    let book_id = Uuid::new_v4();
    let permit = gate
        .try_acquire_normal(ActiveOperationKind::Import)
        .unwrap();
    let attempt = registry.register_with_permit(book_id, CancellationToken::new(), permit);
    let continuation = registry.operation_permit(book_id).unwrap();

    let writer_gate = gate.clone();
    let (writer_queued_tx, writer_queued_rx) = tokio::sync::oneshot::channel();
    let (writer_ready_tx, writer_ready_rx) = tokio::sync::oneshot::channel();
    let (release_writer_tx, release_writer_rx) = tokio::sync::oneshot::channel();
    let writer = tokio::spawn(async move {
        let acquisition = writer_gate.acquire_maintenance();
        tokio::pin!(acquisition);
        // Polling registers the writer's ticket. A scheduler yield alone does
        // not prove the writer ran before the later reader was spawned.
        assert!(futures_util::poll!(acquisition.as_mut()).is_pending());
        writer_queued_tx.send(()).unwrap();
        let exclusive = acquisition.await.unwrap();
        writer_ready_tx.send(()).unwrap();
        release_writer_rx.await.unwrap();
        drop(exclusive);
    });

    writer_queued_rx.await.unwrap();
    let reader_gate = gate.clone();
    let late_reader = tokio::spawn(async move {
        reader_gate
            .acquire_normal(ActiveOperationKind::Learning)
            .await
    });
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(!late_reader.is_finished());
    assert!(!writer.is_finished());

    assert!(registry.remove_if_owner(book_id, attempt));
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(
        !writer.is_finished(),
        "terminal registry removal forced an unlock"
    );
    drop(continuation);
    writer_ready_rx.await.unwrap();
    assert!(
        !late_reader.is_finished(),
        "late reader bypassed queued writer"
    );
    release_writer_tx.send(()).unwrap();
    writer.await.unwrap();
    drop(late_reader.await.unwrap().unwrap());

    let held = gate
        .try_acquire_normal(ActiveOperationKind::Indexing)
        .unwrap();
    gate.shutdown();
    assert!(matches!(
        gate.try_acquire_maintenance(),
        Err(GateAcquireError::ShuttingDown)
    ));
    assert!(
        gate.status()
            .active_operations
            .iter()
            .any(
                |operation| operation.kind == ActiveOperationKind::Indexing && operation.count == 1
            )
    );
    drop(held);
}

#[test]
fn backup_restore_and_clear_recover_across_real_reopen_without_mixed_trees() {
    for boundary in [
        BackupBoundary::DatabaseSnapshotReady,
        BackupBoundary::SourcesValidated,
        BackupBoundary::HeaderWritten,
        BackupBoundary::EntriesWritten,
        BackupBoundary::ArchiveSynced,
        BackupBoundary::ArchiveVerified,
        BackupBoundary::BeforePublish,
    ] {
        let fixture = Fixture::new();
        let database = Database::open(&fixture.paths.database).unwrap();
        tauri::async_runtime::block_on(seed_backup_book(database.pool(), &fixture.paths));
        let destination = fixture.temporary.path().join("faulted.tlbackup");
        let service = BackupService::with_fault_injector(
            database.pool().clone(),
            fixture.paths.clone(),
            MaintenanceGate::default(),
            Arc::new(FailBackup(boundary)),
        );
        tauri::async_runtime::block_on(service.create_backup(destination.clone())).unwrap_err();
        assert!(!destination.exists());
        assert_no_backup_temps(fixture.temporary.path());
        tauri::async_runtime::block_on(database.pool().close());
        drop(database);
        let reopened = Database::open(&fixture.paths.database).unwrap();
        assert_eq!(
            tauri::async_runtime::block_on(
                sqlx::query_scalar::<_, i64>(
                    "SELECT COUNT(*) FROM books WHERE import_status = 'ready'",
                )
                .fetch_one(reopened.pool())
            )
            .unwrap(),
            1
        );
    }

    let fixture = Fixture::new();
    let credentials = Arc::new(MemoryCredentialStore::new());
    let database = Database::open(&fixture.paths.database).unwrap();
    tauri::async_runtime::block_on(async {
        seed_backup_book(database.pool(), &fixture.paths).await;
        sqlx::query("UPDATE app_settings SET ui_language = 'zh-TW' WHERE id = 1")
            .execute(database.pool())
            .await
            .unwrap();
    });
    let archive = fixture.temporary.path().join("durable.tlbackup");
    tauri::async_runtime::block_on(
        BackupService::new(
            database.pool().clone(),
            fixture.paths.clone(),
            MaintenanceGate::default(),
        )
        .create_backup(archive.clone()),
    )
    .unwrap();
    tauri::async_runtime::block_on(async {
        sqlx::query("UPDATE app_settings SET ui_language = 'en' WHERE id = 1")
            .execute(database.pool())
            .await
            .unwrap();
        database.pool().close().await;
    });
    drop(database);

    let staged = tauri::async_runtime::block_on(
        RestoreService::new(
            fixture.paths.clone(),
            MaintenanceGate::default(),
            credentials.clone(),
        )
        .stage_restore(archive.clone()),
    )
    .unwrap();
    assert!(staged.restart_required);
    let restore_fault = FailRestore::new(RestoreBoundary::AfterOldMove(1));
    assert!(recover_pending_restore_with_fault(&fixture.paths.root, &restore_fault).is_err());
    assert!(recover_pending_restore(&fixture.paths.root).unwrap());
    assert!(!recover_pending_restore(&fixture.paths.root).unwrap());
    let restored = Database::open(&fixture.paths.database).unwrap();
    assert_eq!(
        tauri::async_runtime::block_on(
            sqlx::query_scalar::<_, String>("SELECT ui_language FROM app_settings WHERE id = 1",)
                .fetch_one(restored.pool())
        )
        .unwrap(),
        "zh-TW"
    );

    tauri::async_runtime::block_on(async {
        credentials
            .set(
                &format!("textbooklens/{}", Uuid::new_v4()),
                SecretString::from(PRIVATE_KEY.to_owned()),
            )
            .await
            .unwrap();
    });
    let clear = ClearAllDataService::with_fault_injector(
        restored.pool().clone(),
        fixture.paths.clone(),
        MaintenanceGate::default(),
        credentials.clone(),
        Arc::new(FailClear::new(ClearBoundary::AfterCredentialDelete(0))),
    );
    tauri::async_runtime::block_on(
        clear.clear_all_data(CLEAR_ALL_DATA_CONFIRMATION_PHRASE.to_owned()),
    )
    .unwrap_err();
    tauri::async_runtime::block_on(restored.pool().close());
    drop(restored);
    assert_eq!(
        tauri::async_runtime::block_on(recover_pending_clear(
            &fixture.paths.root,
            credentials.as_ref(),
        ))
        .unwrap(),
        ClearStartupOutcome::Cleared
    );
    assert_eq!(
        tauri::async_runtime::block_on(recover_pending_clear(
            &fixture.paths.root,
            credentials.as_ref(),
        ))
        .unwrap(),
        ClearStartupOutcome::NoPending
    );
    assert!(!fixture.paths.database.exists());
    assert!(
        tauri::async_runtime::block_on(credentials.list_textbooklens_keys())
            .unwrap()
            .is_empty()
    );
    assert!(
        archive.exists(),
        "external backup was touched by clear recovery"
    );
}

#[test]
fn delete_trash_recovery_converges_before_and_after_database_commit() {
    let post_commit = DeleteBoundary::fault_matrix(1)
        .into_iter()
        .find(|boundary| boundary.is_after_database_commit())
        .unwrap();
    for boundary in [DeleteBoundary::AfterStageMove(0), post_commit] {
        let fixture = Fixture::new();
        let database = Database::open(&fixture.paths.database).unwrap();
        let book_id =
            tauri::async_runtime::block_on(seed_backup_book(database.pool(), &fixture.paths));
        let service = DeleteBookService::new(
            database.pool().clone(),
            fixture.paths.clone(),
            MaintenanceGate::default(),
        );
        let error = tauri::async_runtime::block_on(
            service.delete_book_with_fault(book_id, &FailDelete::new(boundary)),
        )
        .unwrap_err();
        assert_eq!(error.injected_boundary(), Some(boundary));
        drop(service);
        tauri::async_runtime::block_on(database.pool().close());
        drop(database);

        let reopened = Database::open(&fixture.paths.database).unwrap();
        tauri::async_runtime::block_on(async {
            recover_pending_deletions_async(reopened.pool(), &fixture.paths)
                .await
                .unwrap();
            recover_pending_deletions_async(reopened.pool(), &fixture.paths)
                .await
                .unwrap();
            let exists: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM books WHERE id = ?")
                .bind(book_id.to_string())
                .fetch_one(reopened.pool())
                .await
                .unwrap();
            assert_eq!(exists == 0, boundary.is_after_database_commit());
            assert_eq!(
                fixture.paths.books.join(book_id.to_string()).exists(),
                !boundary.is_after_database_commit()
            );
            if exists == 1 {
                DeleteBookService::new(
                    reopened.pool().clone(),
                    fixture.paths.clone(),
                    MaintenanceGate::default(),
                )
                .delete_book(book_id)
                .await
                .unwrap();
            }
        });
        assert_no_delete_artifacts(&fixture.paths.root);
    }
}

struct Fixture {
    temporary: TempDir,
    paths: AppPaths,
}

impl Fixture {
    fn new() -> Self {
        let temporary = TempDir::new().unwrap();
        let prepared =
            prepare_app_data_paths(&temporary.path().join(APP_DATA_DIRECTORY_NAME)).unwrap();
        Self {
            paths: AppPaths {
                root: prepared.root,
                books: prepared.books,
                cache: prepared.cache,
                logs: prepared.logs,
                database: prepared.database,
            },
            temporary,
        }
    }
}

struct Seeded {
    import_ids: [Uuid; 4],
    durable: DurableSeed,
    pages: Vec<IndexPageOwnership>,
    partial_request: Uuid,
    pending_remote: Uuid,
    marker_remote: Uuid,
}

#[derive(Clone, Copy)]
struct DurableSeed {
    book_id: Uuid,
    profile_id: Uuid,
    section_id: Uuid,
    conversation_id: Uuid,
    note_id: Uuid,
}

async fn seed_interrupted_imports(pool: &SqlitePool, paths: &AppPaths) -> [Uuid; 4] {
    let ids = [
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
    ];
    for (index, (id, status)) in ids
        .iter()
        .zip(["queued", "copying", "parsing", "indexing"])
        .enumerate()
    {
        let (sha, stored) = if matches!(status, "queued" | "copying") {
            (None, None)
        } else {
            (
                Some(if index == 2 { "c" } else { "f" }.repeat(64)),
                Some(format!("books/{id}/original.pdf")),
            )
        };
        sqlx::query(
            "INSERT INTO books (id, sha256, title, format, original_filename, stored_path, import_status, created_at, updated_at) VALUES (?, ?, 'Interrupted synthetic import', 'pdf', 'synthetic.pdf', ?, ?, ?, ?)",
        )
        .bind(id.to_string())
        .bind(sha)
        .bind(stored)
        .bind(status)
        .bind(TIME)
        .bind(TIME)
        .execute(pool)
        .await
        .unwrap();
        let directory = paths.books.join(id.to_string());
        fs::create_dir_all(directory.join("derived")).unwrap();
        if matches!(status, "queued" | "copying") {
            fs::write(directory.join("original.pdf.partial"), b"partial copy").unwrap();
        } else {
            fs::write(directory.join("original.pdf"), b"committed app copy").unwrap();
            fs::write(
                directory.join("derived/document.html.partial"),
                b"partial parse",
            )
            .unwrap();
        }
    }
    let orphan = paths.books.join(Uuid::new_v4().to_string());
    fs::create_dir_all(&orphan).unwrap();
    fs::write(orphan.join("original.pdf"), b"orphan").unwrap();
    ids
}

async fn seed_durable_book(pool: &SqlitePool, paths: &AppPaths) -> DurableSeed {
    let book_id = Uuid::new_v4();
    let profile_id = Uuid::new_v4();
    let section_id = Uuid::new_v4();
    let conversation_id = Uuid::new_v4();
    let note_id = Uuid::new_v4();
    let locator = DocumentLocator::pdf(1, 1, None).unwrap();
    let locator_json = serde_json::to_string(&locator).unwrap();
    let anchor_json = serde_json::to_string(&ContentAnchor::from(SelectionAnchor {
        locator: locator.clone(),
        quote: TextQuote::new(
            "durable selection".to_owned(),
            "before".to_owned(),
            "after".to_owned(),
        )
        .unwrap(),
        section_id: Some(section_id),
    }))
    .unwrap();
    let directory = paths.books.join(book_id.to_string());
    fs::create_dir_all(directory.join("derived")).unwrap();
    fs::write(directory.join("original.pdf"), b"durable synthetic source").unwrap();
    fs::write(
        directory.join("derived/document.html"),
        b"<p>durable local reader text</p>",
    )
    .unwrap();
    sqlx::query(
        "INSERT INTO provider_profiles (id, provider_kind, display_name, model_id, context_window_tokens, created_at, updated_at, validated_at) VALUES (?, 'openai', 'Synthetic profile', 'synthetic-model', 32000, ?, ?, ?)",
    )
    .bind(profile_id.to_string())
    .bind(TIME)
    .bind(TIME)
    .bind(TIME)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO books (id, sha256, title, format, original_filename, stored_path, import_status, created_at, updated_at) VALUES (?, ?, 'Durable synthetic book', 'pdf', 'durable.pdf', ?, 'ready', ?, ?)",
    )
    .bind(book_id.to_string())
    .bind("d".repeat(64))
    .bind(format!("books/{book_id}/original.pdf"))
    .bind(TIME)
    .bind(TIME)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO sections (id, book_id, ordinal, title, locator_json) VALUES (?, ?, 0, 'Durable section', ?)",
    )
    .bind(section_id.to_string())
    .bind(book_id.to_string())
    .bind(&locator_json)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO blocks (id, book_id, section_id, ordinal, kind, plain_text, locator_json) VALUES (?, ?, ?, 0, 'paragraph', 'durable local block', ?)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(book_id.to_string())
    .bind(section_id.to_string())
    .bind(&locator_json)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO search_chunks (id, book_id, section_id, ordinal, text, locator_json, token_estimate) VALUES (?, ?, ?, 0, 'durable local searchable phrase', ?, 4)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(book_id.to_string())
    .bind(section_id.to_string())
    .bind(&locator_json)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO conversations (id, book_id, section_id, scope, anchor_kind, anchor_json, selected_text, created_at, updated_at) VALUES (?, ?, ?, 'selection', 'text', ?, 'durable selection', ?, ?)",
    )
    .bind(conversation_id.to_string())
    .bind(book_id.to_string())
    .bind(section_id.to_string())
    .bind(&anchor_json)
    .bind(TIME)
    .bind(TIME)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO messages (id, conversation_id, ordinal, role, action, content, created_at) VALUES (?, ?, 0, 'user', 'explain', 'durable question', ?)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(conversation_id.to_string())
    .bind(TIME)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO messages (id, conversation_id, ordinal, role, action, content, provider_id, model_id, citations_json, created_at) VALUES (?, ?, 1, 'assistant', 'explain', 'durable answer', ?, 'synthetic-model', '[]', ?)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(conversation_id.to_string())
    .bind(profile_id.to_string())
    .bind(TIME)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO annotations (id, book_id, section_id, kind, anchor_json, selected_text, conversation_id, revision, created_at, updated_at) VALUES (?, ?, ?, 'ai_conversation', ?, 'durable selection', ?, 1, ?, ?)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(book_id.to_string())
    .bind(section_id.to_string())
    .bind(&anchor_json)
    .bind(conversation_id.to_string())
    .bind(TIME)
    .bind(TIME)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO annotations (id, book_id, section_id, kind, anchor_json, selected_text, note_text, revision, created_at, updated_at) VALUES (?, ?, ?, 'note', ?, 'durable selection', 'durable note', 1, ?, ?)",
    )
    .bind(note_id.to_string())
    .bind(book_id.to_string())
    .bind(section_id.to_string())
    .bind(&anchor_json)
    .bind(TIME)
    .bind(TIME)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "UPDATE app_settings SET ui_language = 'en', theme = 'dark', first_reader_hint_completed = 1 WHERE id = 1",
    )
    .execute(pool)
    .await
    .unwrap();
    DurableSeed {
        book_id,
        profile_id,
        section_id,
        conversation_id,
        note_id,
    }
}

async fn seed_page_pipeline(
    pool: &SqlitePool,
    book_id: Uuid,
    profile_id: Uuid,
) -> Vec<IndexPageOwnership> {
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
    for page_number in 1..=6 {
        pages.push(
            state::queue(pool, run_id, page_number, IndexQualityReason::NoText, None)
                .await
                .unwrap(),
        );
    }
    pages
}

async fn drive_to_sending(pool: &SqlitePool, page: IndexPageOwnership) {
    let attempt = page.attempt_id.unwrap();
    state::claim_render(pool, page.page_id, attempt)
        .await
        .unwrap();
    state::mark_rendered(pool, page.page_id, attempt, &"a".repeat(64))
        .await
        .unwrap();
    state::claim_send(pool, page.page_id, attempt)
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

async fn commit_page(pool: &SqlitePool, page: IndexPageOwnership, number: u32, text: &str) {
    let validated = ValidatedPage {
        page_number: number,
        blocks: vec![ValidatedBlock {
            ordinal: 0,
            kind: IndexPageBlockKind::Paragraph,
            plain_text: Some(text.to_owned()),
            latex: None,
            table_cells: None,
            visual_description: None,
            bounds: Some(NormalizedRect::new(0.1, 0.1, 0.8, 0.2).unwrap()),
            source: ContentSource::AiTranscribed,
        }],
        review_reason: None,
    };
    commit_validated_page(
        pool,
        PageCommitRequest {
            page_id: page.page_id,
            attempt_id: page.attempt_id.unwrap(),
            page: &validated,
        },
        &CancellationToken::new(),
    )
    .await
    .unwrap();
}

async fn set_page_time(pool: &SqlitePool, page_id: Uuid, time: &str) {
    sqlx::query("UPDATE index_pages SET updated_at = ? WHERE id = ?")
        .bind(time)
        .bind(page_id.to_string())
        .execute(pool)
        .await
        .unwrap();
}

async fn install_deleted_marker(
    pool: &SqlitePool,
    credentials: &dyn CredentialStore,
    resource_id: Uuid,
) {
    let reference: String = sqlx::query_scalar(
        "SELECT encrypted_reference FROM provider_remote_resources WHERE id = ?",
    )
    .bind(resource_id.to_string())
    .fetch_one(pool)
    .await
    .unwrap();
    let vault_id = reference.strip_prefix("enc:v1:keyring:").unwrap();
    let attempt = Uuid::new_v4();
    sqlx::query(
        "UPDATE provider_remote_resources SET cleanup_status = 'cleaning', cleanup_attempt_id = ?, cleanup_attempt_count = 1, last_cleanup_at = ?, updated_at = ? WHERE id = ?",
    )
    .bind(attempt.to_string())
    .bind(TIME)
    .bind(TIME)
    .bind(resource_id.to_string())
    .execute(pool)
    .await
    .unwrap();
    credentials
        .set(
            &format!("textbooklens/remote-resource/{vault_id}"),
            SecretString::from("textbooklens:remote-resource-deleted:v1"),
        )
        .await
        .unwrap();
}

async fn assert_import_recovery(pool: &SqlitePool, paths: &AppPaths, ids: &[Uuid; 4]) {
    for (id, original) in ids.iter().zip(["queued", "copying", "parsing", "indexing"]) {
        let row = sqlx::query(
            "SELECT import_status, import_error_code, import_error_stage, sha256, stored_path FROM books WHERE id = ?",
        )
        .bind(id.to_string())
        .fetch_one(pool)
        .await
        .unwrap();
        assert_eq!(row.get::<String, _>("import_status"), "failed");
        assert_eq!(
            row.get::<String, _>("import_error_code"),
            "IMPORT_CANCELLED"
        );
        assert_eq!(
            row.get::<String, _>("import_error_stage"),
            if matches!(original, "queued" | "copying") {
                "copying"
            } else {
                original
            }
        );
        let directory = paths.books.join(id.to_string());
        if matches!(original, "queued" | "copying") {
            assert!(!directory.exists());
            assert!(row.get::<Option<String>, _>("sha256").is_none());
            assert!(row.get::<Option<String>, _>("stored_path").is_none());
        } else {
            assert!(directory.join("original.pdf").is_file());
            assert!(!directory.join("derived/document.html.partial").exists());
        }
    }
}

async fn assert_page_recovery(pool: &SqlitePool, pages: &[IndexPageOwnership]) {
    let statuses: Vec<(String, i64, i64)> = sqlx::query_as(
        "SELECT status, attempt_count, content_version FROM index_pages WHERE run_id = ? ORDER BY page_number",
    )
    .bind(pages[0].run_id.to_string())
    .fetch_all(pool)
    .await
    .unwrap();
    assert_eq!(statuses[0].0, "queued");
    assert_eq!(statuses[1].0, "queued");
    assert_eq!(statuses[2].0, "failed");
    assert_eq!(statuses[3].0, "failed");
    assert_eq!(statuses[4], ("indexed".to_owned(), 1, 1));
    assert_eq!(statuses[5], ("queued".to_owned(), 1, 0));
}

async fn assert_committed_state(
    pool: &SqlitePool,
    durable: &DurableSeed,
    indexed: IndexPageOwnership,
) {
    let counts: (i64, i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT COUNT(*) FROM conversations WHERE id = ?), (SELECT COUNT(*) FROM messages WHERE conversation_id = ?), (SELECT COUNT(*) FROM annotations WHERE id = ?), (SELECT COUNT(*) FROM index_corrections WHERE page_id = ?)",
    )
    .bind(durable.conversation_id.to_string())
    .bind(durable.conversation_id.to_string())
    .bind(durable.note_id.to_string())
    .bind(indexed.page_id.to_string())
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(counts, (1, 2, 1, 1));
    let history = load_selection_conversation(pool, durable.book_id, durable.conversation_id)
        .await
        .unwrap();
    assert_eq!(history.messages.len(), 2);
    assert_eq!(history.messages[1].content, "durable answer");
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT ui_language FROM app_settings WHERE id = 1")
            .fetch_one(pool)
            .await
            .unwrap(),
        "en"
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT title FROM sections WHERE id = ?")
            .bind(durable.section_id.to_string())
            .fetch_one(pool)
            .await
            .unwrap(),
        "Durable section"
    );
    assert_eq!(
        search_book(pool, durable.book_id, "durable local searchable", 20)
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        search_book(pool, durable.book_id, "durable corrected page", 20)
            .await
            .unwrap()
            .len(),
        1
    );
    let leaked: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM messages WHERE content LIKE '%' || ? || '%'")
            .bind(PARTIAL_STREAM)
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(leaked, 0);
}

async fn remote_status(pool: &SqlitePool, resource_id: Uuid) -> String {
    sqlx::query_scalar("SELECT cleanup_status FROM provider_remote_resources WHERE id = ?")
        .bind(resource_id.to_string())
        .fetch_one(pool)
        .await
        .unwrap()
}

#[derive(Default)]
struct CountingCleaner {
    calls: AtomicUsize,
}

#[async_trait]
impl RemoteResourceCleaner for CountingCleaner {
    async fn delete(
        &self,
        _pool: &SqlitePool,
        _profile_id: Uuid,
        handle: &RemoteCleanupHandle,
        _cancel: CancellationToken,
    ) -> AppResult<()> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        assert_eq!(handle.opaque_id().expose_secret(), OPAQUE_REMOTE);
        Ok(())
    }
}

async fn seed_backup_book(pool: &SqlitePool, paths: &AppPaths) -> Uuid {
    let book_id = Uuid::new_v4();
    let directory = paths.books.join(book_id.to_string());
    fs::create_dir_all(directory.join("derived")).unwrap();
    let source = b"synthetic backup source";
    fs::write(directory.join("original.pdf"), source).unwrap();
    fs::write(
        directory.join("derived/document.html"),
        b"<p>synthetic backup derived</p>",
    )
    .unwrap();
    sqlx::query(
        "INSERT INTO books (id, sha256, title, format, original_filename, stored_path, import_status, created_at, updated_at) VALUES (?, ?, 'Backup synthetic book', 'pdf', 'backup.pdf', ?, 'ready', ?, ?)",
    )
    .bind(book_id.to_string())
    .bind(hex(Sha256::digest(source)))
    .bind(format!("books/{book_id}/original.pdf"))
    .bind(TIME)
    .bind(TIME)
    .execute(pool)
    .await
    .unwrap();
    book_id
}

fn hex(bytes: impl AsRef<[u8]>) -> String {
    bytes
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn assert_no_backup_temps(parent: &Path) {
    assert!(fs::read_dir(parent).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".textbooklens-backup-")
    }));
}

fn assert_no_delete_artifacts(root: &Path) {
    for name in ["delete-journal", "delete-trash"] {
        let path = root.join(name);
        assert!(
            !path.exists() || fs::read_dir(path).unwrap().next().is_none(),
            "pending delete artifact remained"
        );
    }
}

struct FailBackup(BackupBoundary);

impl BackupFaultInjector for FailBackup {
    fn checkpoint(&self, boundary: BackupBoundary) -> Result<(), BackupInjectedFailure> {
        if boundary == self.0 {
            Err(BackupInjectedFailure)
        } else {
            Ok(())
        }
    }
}

struct FailRestore {
    boundary: RestoreBoundary,
    fired: AtomicBool,
}

impl FailRestore {
    const fn new(boundary: RestoreBoundary) -> Self {
        Self {
            boundary,
            fired: AtomicBool::new(false),
        }
    }
}

impl RestoreFaultInjector for FailRestore {
    fn checkpoint(&self, boundary: RestoreBoundary) -> Result<(), RestoreInjectedCrash> {
        if boundary == self.boundary && !self.fired.swap(true, Ordering::SeqCst) {
            Err(RestoreInjectedCrash)
        } else {
            Ok(())
        }
    }
}

struct FailClear {
    boundary: ClearBoundary,
    fired: AtomicBool,
}

impl FailClear {
    const fn new(boundary: ClearBoundary) -> Self {
        Self {
            boundary,
            fired: AtomicBool::new(false),
        }
    }
}

impl ClearFaultInjector for FailClear {
    fn checkpoint(&self, boundary: ClearBoundary) -> Result<(), ClearInjectedCrash> {
        if boundary == self.boundary && !self.fired.swap(true, Ordering::SeqCst) {
            Err(ClearInjectedCrash)
        } else {
            Ok(())
        }
    }
}

struct FailDelete {
    boundary: DeleteBoundary,
    fired: AtomicBool,
}

impl FailDelete {
    const fn new(boundary: DeleteBoundary) -> Self {
        Self {
            boundary,
            fired: AtomicBool::new(false),
        }
    }
}

impl DeleteFaultInjector for FailDelete {
    fn checkpoint(&self, boundary: DeleteBoundary) -> Result<(), DeleteInjectedCrash> {
        if boundary == self.boundary && !self.fired.swap(true, Ordering::SeqCst) {
            Err(DeleteInjectedCrash)
        } else {
            Ok(())
        }
    }
}
