use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use async_trait::async_trait;
use secrecy::{ExposeSecret, SecretString};
use sqlx::{Row, SqlitePool};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{header, method, path},
};

use super::remote_cleanup::{
    RemoteResourceCleaner, RuntimeRemoteResourceCleaner, finish_failed_tracking_compensation,
    recover_remote_cleanup_on_startup, safely_dispose_failed_resource,
    store_extraction_remote_handle, store_remote_handle, sweep_remote_resources,
};
use crate::{
    ai::registry::ProviderCapabilityRegistry,
    credentials::CredentialStore,
    db::{
        Database,
        indexing::{CreateIndexRun, create_index_run},
        providers,
    },
    domain::{IndexQualityReason, KimiApiRegion, ProviderKind, RemoteCleanupHandle},
    errors::{AppError, AppErrorCode, AppResult},
    indexing::state,
};

#[derive(Default)]
struct TestStore {
    values: Mutex<HashMap<String, SecretString>>,
    fail_gets: AtomicUsize,
}

impl TestStore {
    fn entry_count(&self) -> usize {
        self.values.lock().unwrap().len()
    }

    fn only_value(&self) -> String {
        self.values
            .lock()
            .unwrap()
            .values()
            .next()
            .unwrap()
            .expose_secret()
            .to_owned()
    }
}

#[async_trait]
impl CredentialStore for TestStore {
    async fn set(&self, key: &str, value: SecretString) -> AppResult<()> {
        self.values.lock().unwrap().insert(key.to_owned(), value);
        Ok(())
    }

    async fn get(&self, key: &str) -> AppResult<SecretString> {
        if self.fail_gets.load(Ordering::SeqCst) > 0 {
            self.fail_gets.fetch_sub(1, Ordering::SeqCst);
            return Err(AppError::credential_store("synthetic unavailable vault"));
        }
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
    cancel: bool,
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
        if self.cancel {
            return Err(AppError::new(AppErrorCode::ImportCancelled));
        }
        if self.failures_remaining.load(Ordering::SeqCst) > 0 {
            self.failures_remaining.fetch_sub(1, Ordering::SeqCst);
            return Err(AppError::new(AppErrorCode::ProviderUnavailable));
        }
        Ok(())
    }
}

#[test]
fn handle_is_vaulted_and_cleanup_is_claimed_at_most_once() {
    let fixture = RemoteFixture::new();
    tauri::async_runtime::block_on(async {
        let store = Arc::new(TestStore::default());
        let sentinel = "opaque-remote-id-SENTINEL";
        let resource_id = store_remote_handle(
            fixture.database.pool(),
            store.as_ref(),
            fixture.page_id,
            &RemoteCleanupHandle::new(
                ProviderKind::OpenAi,
                SecretString::from(sentinel.to_owned()),
            ),
        )
        .await
        .unwrap();
        let row = sqlx::query(
            "SELECT encrypted_reference, cleanup_status FROM provider_remote_resources WHERE id = ?",
        )
        .bind(resource_id.to_string())
        .fetch_one(fixture.database.pool())
        .await
        .unwrap();
        let reference: String = row.get("encrypted_reference");
        assert!(reference.starts_with("enc:v1:keyring:"));
        assert!(!reference.contains(sentinel));
        assert_eq!(row.get::<String, _>("cleanup_status"), "pending");
        assert_eq!(store.only_value(), sentinel);

        let cleaner = Arc::new(FakeCleaner::default());
        let first = sweep_remote_resources(
            fixture.database.pool(),
            store.as_ref(),
            cleaner.as_ref(),
            CancellationToken::new(),
        );
        let second = sweep_remote_resources(
            fixture.database.pool(),
            store.as_ref(),
            cleaner.as_ref(),
            CancellationToken::new(),
        );
        let (first, second) = tokio::join!(first, second);
        first.unwrap();
        second.unwrap();
        assert_eq!(cleaner.calls.load(Ordering::SeqCst), 1);
        assert_eq!(store.entry_count(), 0);
        assert_eq!(
            cleanup_status(fixture.database.pool(), resource_id).await,
            "succeeded"
        );
    });
}

#[test]
fn failed_delete_waits_for_an_explicit_retry_after_local_only_startup_recovery() {
    let fixture = RemoteFixture::new();
    tauri::async_runtime::block_on(async {
        let store = TestStore::default();
        let resource_id = fixture.track(&store, "opaque-retry-sentinel").await;
        let cleaner = FakeCleaner::default();
        cleaner.failures_remaining.store(1, Ordering::SeqCst);

        let first = sweep_remote_resources(
            fixture.database.pool(),
            &store,
            &cleaner,
            CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(first.failed, 1);
        assert_eq!(
            cleanup_status(fixture.database.pool(), resource_id).await,
            "failed"
        );
        assert_eq!(store.entry_count(), 1);

        let startup = recover_remote_cleanup_on_startup(fixture.database.pool(), &store)
            .await
            .unwrap();
        assert_eq!(startup, Default::default());
        assert_eq!(cleaner.calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            cleanup_status(fixture.database.pool(), resource_id).await,
            "failed"
        );
        assert_eq!(store.entry_count(), 1);

        let explicit_retry = sweep_remote_resources(
            fixture.database.pool(),
            &store,
            &cleaner,
            CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(explicit_retry.succeeded, 1);
        assert_eq!(cleaner.calls.load(Ordering::SeqCst), 2);
        assert_eq!(
            cleanup_status(fixture.database.pool(), resource_id).await,
            "succeeded"
        );
        assert_eq!(store.entry_count(), 0);
    });
}

#[test]
fn startup_marker_recovery_never_repeats_a_completed_provider_delete() {
    let fixture = RemoteFixture::new();
    tauri::async_runtime::block_on(async {
        let store = TestStore::default();
        let resource_id = fixture.track(&store, "opaque-before-crash-sentinel").await;
        let reference: String = sqlx::query_scalar(
            "SELECT encrypted_reference FROM provider_remote_resources WHERE id = ?",
        )
        .bind(resource_id.to_string())
        .fetch_one(fixture.database.pool())
        .await
        .unwrap();
        let vault_id = reference.strip_prefix("enc:v1:keyring:").unwrap();
        let key = format!("textbooklens/remote-resource/{vault_id}");
        let attempt_id = Uuid::new_v4();
        sqlx::query(
            "UPDATE provider_remote_resources SET cleanup_status = 'cleaning', cleanup_attempt_id = ?, cleanup_attempt_count = 1, last_cleanup_at = '2026-08-04T00:00:01.000Z', updated_at = '2026-08-04T00:00:01.000Z' WHERE id = ? AND cleanup_status = 'pending'",
        )
        .bind(attempt_id.to_string())
        .bind(resource_id.to_string())
        .execute(fixture.database.pool())
        .await
        .unwrap();
        store
            .set(
                &key,
                SecretString::from("textbooklens:remote-resource-deleted:v1"),
            )
            .await
            .unwrap();
        let summary = recover_remote_cleanup_on_startup(fixture.database.pool(), &store)
            .await
            .unwrap();
        assert_eq!(summary.recovered_success_markers, 1);
        assert_eq!(
            cleanup_status(fixture.database.pool(), resource_id).await,
            "succeeded"
        );
        assert_eq!(
            recover_remote_cleanup_on_startup(fixture.database.pool(), &store)
                .await
                .unwrap(),
            Default::default()
        );
    });
}

#[test]
fn missing_or_unavailable_vault_never_claims_cleanup_success() {
    let fixture = RemoteFixture::new();
    tauri::async_runtime::block_on(async {
        let store = TestStore::default();
        let resource_id = fixture.track(&store, "opaque-missing-sentinel").await;
        let reference: String = sqlx::query_scalar(
            "SELECT encrypted_reference FROM provider_remote_resources WHERE id = ?",
        )
        .bind(resource_id.to_string())
        .fetch_one(fixture.database.pool())
        .await
        .unwrap();
        let vault_id = reference.strip_prefix("enc:v1:keyring:").unwrap();
        store
            .delete(&format!("textbooklens/remote-resource/{vault_id}"))
            .await
            .unwrap();
        let cleaner = FakeCleaner::default();
        let summary = sweep_remote_resources(
            fixture.database.pool(),
            &store,
            &cleaner,
            CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(summary.failed, 1);
        assert_eq!(cleaner.calls.load(Ordering::SeqCst), 0);
        let row = sqlx::query(
            "SELECT cleanup_status, cleanup_attempt_id, safe_error_code FROM provider_remote_resources WHERE id = ?",
        )
        .bind(resource_id.to_string())
        .fetch_one(fixture.database.pool())
        .await
        .unwrap();
        assert_eq!(row.get::<String, _>("cleanup_status"), "failed");
        assert_eq!(
            row.get::<String, _>("safe_error_code"),
            "REMOTE_AUTH_REQUIRED"
        );
        let attempt_id = Uuid::parse_str(&row.get::<String, _>("cleanup_attempt_id")).unwrap();
        let denied = safely_dispose_failed_resource(
            fixture.database.pool(),
            &store,
            resource_id,
            attempt_id,
            false,
        )
        .await
        .unwrap_err();
        assert_eq!(denied.code, AppErrorCode::InvalidInput);
        safely_dispose_failed_resource(
            fixture.database.pool(),
            &store,
            resource_id,
            attempt_id,
            true,
        )
        .await
        .unwrap();
        assert_eq!(
            cleanup_status(fixture.database.pool(), resource_id).await,
            "safely_disposed"
        );
    });
}

#[test]
fn db_tracking_failure_is_compensated_without_persisting_opaque_id() {
    let fixture = RemoteFixture::new();
    tauri::async_runtime::block_on(async {
        sqlx::query(
            "CREATE TRIGGER synthetic_remote_insert_failure BEFORE INSERT ON provider_remote_resources BEGIN SELECT RAISE(ABORT, 'synthetic remote insert failure'); END",
        )
        .execute(fixture.database.pool())
        .await
        .unwrap();
        let store = TestStore::default();
        let sentinel = "opaque-db-failure-sentinel";
        let failure = store_remote_handle(
            fixture.database.pool(),
            &store,
            fixture.page_id,
            &RemoteCleanupHandle::new(
                ProviderKind::OpenAi,
                SecretString::from(sentinel.to_owned()),
            ),
        )
        .await
        .unwrap_err();
        assert!(!format!("{failure:?}").contains(sentinel));
        assert_eq!(store.entry_count(), 1);
        finish_failed_tracking_compensation(&store, failure, true).await;
        assert_eq!(store.entry_count(), 0);
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM provider_remote_resources")
            .fetch_one(fixture.database.pool())
            .await
            .unwrap();
        assert_eq!(count, 0);
    });
}

#[test]
fn cancellation_failure_remains_durable_and_does_not_block_local_page_state() {
    let fixture = RemoteFixture::new();
    tauri::async_runtime::block_on(async {
        let store = TestStore::default();
        let resource_id = fixture.track(&store, "opaque-cancel-sentinel").await;
        let cleaner = FakeCleaner {
            cancel: true,
            ..FakeCleaner::default()
        };
        let summary = sweep_remote_resources(
            fixture.database.pool(),
            &store,
            &cleaner,
            CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(summary.failed, 1);
        assert_eq!(
            cleanup_status(fixture.database.pool(), resource_id).await,
            "failed"
        );
        let page_status: String = sqlx::query_scalar("SELECT status FROM index_pages WHERE id = ?")
            .bind(fixture.page_id.to_string())
            .fetch_one(fixture.database.pool())
            .await
            .unwrap();
        assert_eq!(page_status, "queued");
    });
}

#[test]
fn kimi_cleanup_uses_the_resource_creation_region_after_profile_region_changes() {
    let temporary = tempfile::tempdir().unwrap();
    let database = Database::open(temporary.path().join("kimi-region-cleanup.sqlite3")).unwrap();
    tauri::async_runtime::block_on(async {
        let timestamp = "2026-08-08T00:00:00.000Z";
        let book_id = Uuid::new_v4();
        let profile_id = Uuid::new_v4();
        let extraction_id = Uuid::new_v4();
        let store = Arc::new(TestStore::default());
        let credential = "synthetic-kimi-cleanup-credential";
        let opaque_id = "file_old_international_region";
        sqlx::query("INSERT INTO books (id, sha256, title, format, original_filename, stored_path, import_status, created_at, updated_at) VALUES (?, ?, 'Kimi cleanup fixture', 'pdf', 'fixture.pdf', 'fixture/source.pdf', 'ready', ?, ?)")
            .bind(book_id.to_string()).bind("b".repeat(64)).bind(timestamp).bind(timestamp)
            .execute(database.pool()).await.unwrap();
        sqlx::query("INSERT INTO provider_profiles (id, provider_kind, display_name, model_id, context_window_tokens, created_at, updated_at, validated_at, kimi_api_region) VALUES (?, 'kimi', 'Kimi current CN profile', 'kimi-k3', 1000000, ?, ?, ?, 'cn')")
            .bind(profile_id.to_string()).bind(timestamp).bind(timestamp).bind(timestamp)
            .execute(database.pool()).await.unwrap();
        sqlx::query("INSERT INTO book_extractions (id, book_id, source_sha256, provider_profile_id, provider_kind, model_id, extraction_version, credential_fingerprint, status, created_at, updated_at, kimi_api_region) VALUES (?, ?, ?, ?, 'kimi', 'kimi-k3', 'kimi-file-extract-v1', ?, 'processing', ?, ?, 'international')")
            .bind(extraction_id.to_string()).bind(book_id.to_string()).bind("b".repeat(64))
            .bind(profile_id.to_string()).bind("c".repeat(64)).bind(timestamp).bind(timestamp)
            .execute(database.pool()).await.unwrap();
        store
            .set(
                &providers::credential_key(profile_id),
                SecretString::from(credential),
            )
            .await
            .unwrap();
        let resource_id = store_extraction_remote_handle(
            database.pool(),
            store.as_ref(),
            extraction_id,
            &RemoteCleanupHandle::new_kimi(
                SecretString::from(opaque_id),
                KimiApiRegion::International,
            ),
        )
        .await
        .unwrap();
        let stored_region: String = sqlx::query_scalar(
            "SELECT kimi_api_region FROM provider_remote_resources WHERE id = ?",
        )
        .bind(resource_id.to_string())
        .fetch_one(database.pool())
        .await
        .unwrap();
        assert_eq!(stored_region, "international");

        let cn = MockServer::start().await;
        let international = MockServer::start().await;
        Mock::given(method("DELETE"))
            .and(path(format!("/v1/files/{opaque_id}")))
            .and(header("authorization", format!("Bearer {credential}")))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&international)
            .await;
        let cleaner = RuntimeRemoteResourceCleaner::new_for_test(
            store.clone(),
            ProviderCapabilityRegistry::load_embedded().unwrap(),
            &format!("{}/v1", cn.uri()),
            &format!("{}/v1", international.uri()),
        )
        .unwrap();
        let summary = sweep_remote_resources(
            database.pool(),
            store.as_ref(),
            &cleaner,
            CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(summary.succeeded, 1);
        assert!(cn.received_requests().await.unwrap().is_empty());
        assert_eq!(international.received_requests().await.unwrap().len(), 1);
        assert_eq!(
            cleanup_status(database.pool(), resource_id).await,
            "succeeded"
        );
        let local_reference: String = sqlx::query_scalar(
            "SELECT encrypted_reference FROM provider_remote_resources WHERE id = ?",
        )
        .bind(resource_id.to_string())
        .fetch_one(database.pool())
        .await
        .unwrap();
        assert!(!local_reference.contains(opaque_id));
    });
}

struct RemoteFixture {
    _temporary: tempfile::TempDir,
    database: Database,
    page_id: Uuid,
}

impl RemoteFixture {
    fn new() -> Self {
        let temporary = tempfile::tempdir().unwrap();
        let database = Database::open(temporary.path().join("remote-cleanup.sqlite3")).unwrap();
        let page_id = tauri::async_runtime::block_on(async {
            let timestamp = "2026-08-04T00:00:00.000Z";
            let book_id = Uuid::new_v4();
            let profile_id = Uuid::new_v4();
            sqlx::query(
                "INSERT INTO books (id, sha256, title, format, original_filename, stored_path, import_status, created_at, updated_at) VALUES (?, ?, 'Synthetic remote cleanup book', 'pdf', 'remote.pdf', 'books/remote/original.pdf', 'ready', ?, ?)",
            )
            .bind(book_id.to_string())
            .bind("a".repeat(64))
            .bind(timestamp)
            .bind(timestamp)
            .execute(database.pool())
            .await
            .unwrap();
            sqlx::query(
                "INSERT INTO provider_profiles (id, provider_kind, display_name, model_id, context_window_tokens, created_at, updated_at, validated_at) VALUES (?, 'openai', 'Synthetic remote cleanup profile', 'gpt-5.6', 1050000, ?, ?, ?)",
            )
            .bind(profile_id.to_string())
            .bind(timestamp)
            .bind(timestamp)
            .bind(timestamp)
            .execute(database.pool())
            .await
            .unwrap();
            let run_id = create_index_run(
                database.pool(),
                CreateIndexRun {
                    book_id,
                    provider_profile_id: profile_id,
                    analysis_schema_version: "textbooklens.page-analysis.v1".to_owned(),
                    render_version: "pdfjs-render-v1".to_owned(),
                    parser_version: "pdfjs-layout-v1".to_owned(),
                },
            )
            .await
            .unwrap();
            state::queue(database.pool(), run_id, 1, IndexQualityReason::NoText, None)
                .await
                .unwrap()
                .page_id
        });
        Self {
            _temporary: temporary,
            database,
            page_id,
        }
    }

    async fn track(&self, store: &TestStore, opaque: &str) -> Uuid {
        store_remote_handle(
            self.database.pool(),
            store,
            self.page_id,
            &RemoteCleanupHandle::new(ProviderKind::OpenAi, SecretString::from(opaque.to_owned())),
        )
        .await
        .unwrap()
    }
}

async fn cleanup_status(pool: &SqlitePool, resource_id: Uuid) -> String {
    sqlx::query_scalar("SELECT cleanup_status FROM provider_remote_resources WHERE id = ?")
        .bind(resource_id.to_string())
        .fetch_one(pool)
        .await
        .unwrap()
}
