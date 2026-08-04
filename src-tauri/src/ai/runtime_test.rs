use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use futures_util::StreamExt;
use secrecy::{ExposeSecret, SecretString};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

use super::{
    multimodal::stage_vision_asset, registry::ProviderCapabilityRegistry, runtime::ProviderRuntime,
    structured::PAGE_ANALYSIS_SCHEMA_VERSION,
};
use crate::{
    commands::credentials::{
        SaveProviderProfileRequest, replace_provider_profile_credential_impl,
        validate_and_save_provider_profile_impl,
    },
    credentials::CredentialStore,
    db::{Database, providers},
    domain::{
        AiOperation, ImageLimits, ImageMime, ProviderKind, ProviderOperationConsentDecision,
        RemoteCleanupHandle, StructuredPageRequest, UnifiedChatRequest, UnifiedMessage,
        UnifiedRole, UnifiedStreamEvent,
    },
    errors::{AppError, AppErrorCode, AppErrorDto, AppResult},
};

struct SetBarrier {
    call: usize,
    reached: Arc<Notify>,
    release: Arc<Notify>,
}

#[derive(Default)]
struct ScriptedStore {
    values: Mutex<HashMap<String, SecretString>>,
    get_calls: AtomicUsize,
    set_calls: AtomicUsize,
    delete_calls: AtomicUsize,
    fail_get_at: usize,
    fail_set_at: usize,
    fail_delete_at: usize,
    panic_set_at: usize,
    set_barrier: Option<SetBarrier>,
}

impl ScriptedStore {
    fn with_failures(fail_get_at: usize, fail_set_at: usize, fail_delete_at: usize) -> Self {
        Self {
            fail_get_at,
            fail_set_at,
            fail_delete_at,
            ..Self::default()
        }
    }

    fn with_set_barrier(call: usize, reached: Arc<Notify>, release: Arc<Notify>) -> Self {
        Self {
            set_barrier: Some(SetBarrier {
                call,
                reached,
                release,
            }),
            ..Self::default()
        }
    }

    fn with_set_panic(call: usize) -> Self {
        Self {
            panic_set_at: call,
            ..Self::default()
        }
    }

    fn seed(&self, key: &str, value: &str) {
        self.values
            .lock()
            .unwrap()
            .insert(key.to_owned(), SecretString::from(value));
    }

    fn exposed_value(&self, key: &str) -> Option<String> {
        self.values
            .lock()
            .unwrap()
            .get(key)
            .map(|value| value.expose_secret().to_owned())
    }

    fn entry_count(&self) -> usize {
        self.values.lock().unwrap().len()
    }
}

#[async_trait]
impl CredentialStore for ScriptedStore {
    async fn set(&self, key: &str, value: SecretString) -> AppResult<()> {
        let call = self.set_calls.fetch_add(1, Ordering::SeqCst) + 1;
        assert_ne!(call, self.panic_set_at, "synthetic credential worker panic");
        if call == self.fail_set_at {
            return Err(AppError::credential_store(
                "synthetic credential store unavailable",
            ));
        }
        self.values.lock().unwrap().insert(key.to_owned(), value);
        if let Some(barrier) = &self.set_barrier
            && barrier.call == call
        {
            barrier.reached.notify_one();
            barrier.release.notified().await;
        }
        Ok(())
    }

    async fn get(&self, key: &str) -> AppResult<SecretString> {
        let call = self.get_calls.fetch_add(1, Ordering::SeqCst) + 1;
        if call == self.fail_get_at {
            return Err(AppError::credential_store(
                "synthetic credential store unavailable",
            ));
        }
        self.values
            .lock()
            .unwrap()
            .get(key)
            .cloned()
            .ok_or_else(|| AppError::credential_store("synthetic credential missing"))
    }

    async fn delete(&self, key: &str) -> AppResult<()> {
        let call = self.delete_calls.fetch_add(1, Ordering::SeqCst) + 1;
        if call == self.fail_delete_at {
            return Err(AppError::credential_store(
                "synthetic credential store unavailable",
            ));
        }
        self.values.lock().unwrap().remove(key);
        Ok(())
    }
}

#[tokio::test]
async fn runtime_uses_fixed_kind_adapter_and_rejects_non_loopback_or_wrong_models() {
    let store = Arc::new(ScriptedStore::default());
    let registry = ProviderCapabilityRegistry::load_embedded().unwrap();
    let non_loopback = ProviderRuntime::new_for_test(
        store.clone(),
        registry.clone(),
        "https://provider.example.invalid",
    )
    .unwrap_err();
    assert_eq!(non_loopback.code, AppErrorCode::InvalidInput);

    let server = validation_server().await;
    let runtime = ProviderRuntime::new_for_test(store, registry, &server.uri()).unwrap();
    let wrong_owner = runtime
        .validate_candidate(
            ProviderKind::DeepSeek,
            Some("gpt-5.6"),
            SecretString::from("synthetic-runtime-credential"),
            AiOperation::TextLearning,
        )
        .await
        .unwrap_err();
    assert_eq!(wrong_owner.code, AppErrorCode::ModelNotFound);
    let unsupported = runtime
        .validate_candidate(
            ProviderKind::DeepSeek,
            Some("deepseek-v4-flash"),
            SecretString::from("synthetic-runtime-credential"),
            AiOperation::VisionLearning,
        )
        .await
        .unwrap_err();
    assert_eq!(unsupported.stable_code(), "UNSUPPORTED_PROVIDER_CAPABILITY");

    let credential = "synthetic-validated-debug-credential";
    let validated = runtime
        .validate_candidate(
            ProviderKind::OpenAi,
            None,
            SecretString::from(credential),
            AiOperation::TextLearning,
        )
        .await
        .unwrap();
    assert!(!format!("{validated:?}").contains(credential));
    assert!(!format!("{runtime:?}").contains(&server.uri()));
}

#[tokio::test]
async fn runtime_loads_only_the_uuid_key_and_rechecks_before_using_the_adapter() {
    let (_temporary, database) = test_database();
    let store = Arc::new(ScriptedStore::default());
    let profile_id = Uuid::new_v4();
    insert_profile(database.pool(), profile_id, "openai", "gpt-5.6", 32_000).await;
    let key = providers::credential_key(profile_id);
    store.seed(&key, "synthetic-loaded-credential");
    let server = validation_server().await;
    let runtime = ProviderRuntime::new_for_test(
        store,
        ProviderCapabilityRegistry::load_embedded().unwrap(),
        &server.uri(),
    )
    .unwrap();

    let loaded = runtime
        .load(database.pool(), profile_id, AiOperation::TextLearning)
        .await
        .unwrap();
    assert_eq!(loaded.provider_kind(), ProviderKind::OpenAi);
    assert_eq!(loaded.profile().id, profile_id);
    assert_eq!(loaded.operation(), AiOperation::TextLearning);
    let validation = loaded.revalidate().await.unwrap();
    assert_eq!(validation.model, "gpt-5.6");
    assert!(!format!("{loaded:?}").contains("synthetic-loaded-credential"));

    let invalid_id = Uuid::new_v4();
    insert_profile(database.pool(), invalid_id, "deepseek", "gpt-5.6", 32_000).await;
    let error = runtime
        .load(database.pool(), invalid_id, AiOperation::TextLearning)
        .await
        .unwrap_err();
    assert_eq!(error.code, AppErrorCode::ModelNotFound);
}

#[tokio::test]
async fn loaded_provider_streams_only_its_captured_text_model_and_preserves_terminal_events() {
    let (_temporary, database) = test_database();
    let store = Arc::new(ScriptedStore::default());
    let profile_id = Uuid::new_v4();
    insert_profile(database.pool(), profile_id, "openai", "gpt-5.6", 32_000).await;
    store.seed(
        &providers::credential_key(profile_id),
        "synthetic-stream-credential",
    );
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            "event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"synthetic visible\"}\n\nevent: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\",\"usage\":null}}\n\n",
            "text/event-stream",
        ))
        .expect(1)
        .mount(&server)
        .await;
    let runtime = test_runtime(store, &server);
    let loaded = runtime
        .load(database.pool(), profile_id, AiOperation::TextLearning)
        .await
        .unwrap();

    let events = loaded
        .stream_text_learning(test_chat_request("gpt-5.6"), CancellationToken::new())
        .await
        .unwrap()
        .collect::<Vec<_>>()
        .await;
    assert!(events.iter().any(|event| matches!(
        event,
        Ok(UnifiedStreamEvent::TextDelta { text }) if text == "synthetic visible"
    )));
    assert!(matches!(
        events.last(),
        Some(Ok(UnifiedStreamEvent::Completed))
    ));
}

#[tokio::test]
async fn loaded_provider_denies_model_mismatch_locally_without_sensitive_leakage() {
    let (_temporary, database) = test_database();
    let store = Arc::new(ScriptedStore::default());
    let profile_id = Uuid::new_v4();
    insert_profile(database.pool(), profile_id, "openai", "gpt-5.6", 32_000).await;
    let credential_sentinel = "runtime-credential-sentinel";
    let request_sentinel = "runtime-request-sentinel";
    store.seed(&providers::credential_key(profile_id), credential_sentinel);
    let server = MockServer::start().await;
    let runtime = test_runtime(store, &server);
    let loaded = runtime
        .load(database.pool(), profile_id, AiOperation::TextLearning)
        .await
        .unwrap();
    let mut request = test_chat_request("wrong-model");
    request.system = request_sentinel.to_owned();
    request.messages[0].content = request_sentinel.to_owned();

    let error = match loaded
        .stream_text_learning(request, CancellationToken::new())
        .await
    {
        Ok(_) => panic!("model mismatch must be denied before HTTP"),
        Err(error) => error,
    };
    assert_eq!(error.code, AppErrorCode::InvalidInput);
    let surfaces = [
        format!("{error:?}"),
        error.to_string(),
        serde_json::to_string(&AppErrorDto::from(error)).unwrap(),
    ];
    for surface in surfaces {
        assert!(!surface.contains(credential_sentinel));
        assert!(!surface.contains(request_sentinel));
    }
}

#[tokio::test]
async fn loaded_provider_runs_only_captured_structured_model_inline_and_honours_cancellation() {
    let (_temporary, database) = test_database();
    let store = Arc::new(ScriptedStore::default());
    let profile_id = Uuid::new_v4();
    insert_profile(database.pool(), profile_id, "openai", "gpt-5.6", 32_000).await;
    let credential_sentinel = "synthetic-structured-runtime-credential";
    store.seed(&providers::credential_key(profile_id), credential_sentinel);
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            include_str!("../../../fixtures/providers/openai/structured-ok.json"),
            "application/json",
        ))
        .expect(1)
        .mount(&server)
        .await;
    let runtime = test_runtime(store, &server);
    let loaded = runtime
        .load(
            database.pool(),
            profile_id,
            AiOperation::StructuredPageAnalysis,
        )
        .await
        .unwrap();
    let book_id = Uuid::new_v4();

    let outcome = loaded
        .analyze_pages(
            test_structured_request(book_id, "gpt-5.6"),
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(
        outcome.analysis.schema_version,
        PAGE_ANALYSIS_SCHEMA_VERSION
    );
    assert!(outcome.cleanup.is_none());
    assert!(!format!("{outcome:?}").contains("synthetic visible page text"));

    let cancelled = CancellationToken::new();
    cancelled.cancel();
    let error = loaded
        .analyze_pages(test_structured_request(book_id, "gpt-5.6"), cancelled)
        .await
        .unwrap_err();
    assert_eq!(error.code, AppErrorCode::ImportCancelled);

    let mismatch = loaded
        .analyze_pages(
            test_structured_request(book_id, "runtime-request-model-sentinel"),
            CancellationToken::new(),
        )
        .await
        .unwrap_err();
    assert_eq!(mismatch.code, AppErrorCode::InvalidInput);

    let wrong_provider = RemoteCleanupHandle::new(
        ProviderKind::Gemini,
        SecretString::from("runtime-cleanup-mismatch-sentinel"),
    );
    let mismatch = loaded
        .cleanup_remote_resource(&wrong_provider, CancellationToken::new())
        .await
        .unwrap_err();
    assert_eq!(mismatch.code, AppErrorCode::InvalidInput);

    let inline_only = RemoteCleanupHandle::new(
        ProviderKind::OpenAi,
        SecretString::from("runtime-cleanup-inline-sentinel"),
    );
    let unsupported = loaded
        .cleanup_remote_resource(&inline_only, CancellationToken::new())
        .await
        .unwrap_err();
    assert_eq!(unsupported.stable_code(), "UNSUPPORTED_PROVIDER_CAPABILITY");

    let received = server.received_requests().await.unwrap();
    assert_eq!(received.len(), 1, "all denials must happen before HTTP");
    let surfaces = format!("{outcome:?} {error:?} {mismatch:?} {unsupported:?} {loaded:?}");
    for sentinel in [
        credential_sentinel,
        "runtime-request-model-sentinel",
        "runtime-cleanup-mismatch-sentinel",
        "runtime-cleanup-inline-sentinel",
        "synthetic visible page text",
    ] {
        assert!(!surfaces.contains(sentinel));
    }
}

#[tokio::test]
async fn create_validates_then_writes_the_derived_key_and_transaction() {
    let (_temporary, database) = test_database();
    let store = Arc::new(ScriptedStore::default());
    let server = validation_server().await;
    let runtime = test_runtime(store.clone(), &server);
    let profile = validate_and_save_provider_profile_impl(
        save_request("synthetic-create-credential"),
        database.pool(),
        store.clone(),
        &runtime,
    )
    .await
    .unwrap();

    assert_eq!(profile.kind, ProviderKind::OpenAi);
    assert_eq!(profile.model_id, "gpt-5.6");
    assert!(profile.is_active);
    assert_eq!(
        store.exposed_value(&providers::credential_key(profile.id)),
        Some("synthetic-create-credential".to_owned())
    );
    let defaults: (Option<String>, Option<String>, Option<String>) = sqlx::query_as(
        "SELECT active_provider_profile_id, default_learning_profile_id, default_vision_profile_id FROM app_settings WHERE id = 1",
    )
    .fetch_one(database.pool())
    .await
    .unwrap();
    assert_eq!(
        defaults,
        (
            Some(profile.id.to_string()),
            Some(profile.id.to_string()),
            None
        )
    );
    let consents = providers::list_provider_operation_consents(database.pool(), profile.id)
        .await
        .unwrap();
    assert_eq!(consents.len(), 3);
    assert!(
        consents
            .iter()
            .all(|consent| consent.decision == ProviderOperationConsentDecision::Ask)
    );
}

#[tokio::test]
async fn create_database_failure_removes_the_new_key() {
    let (_temporary, database) = test_database();
    install_insert_failure(database.pool()).await;
    let store = Arc::new(ScriptedStore::default());
    let server = validation_server().await;
    let runtime = test_runtime(store.clone(), &server);

    let error = validate_and_save_provider_profile_impl(
        save_request("synthetic-create-rollback"),
        database.pool(),
        store.clone(),
        &runtime,
    )
    .await
    .unwrap_err();
    assert_eq!(error.code, AppErrorCode::DatabaseError);
    assert_eq!(store.entry_count(), 0);
    assert_eq!(profile_count(database.pool()).await, 0);
}

#[tokio::test]
async fn create_cleanup_failure_exposes_only_a_diagnostic_id() {
    let (_temporary, database) = test_database();
    install_insert_failure(database.pool()).await;
    let store = Arc::new(ScriptedStore::with_failures(0, 0, 1));
    let server = validation_server().await;
    let runtime = test_runtime(store.clone(), &server);

    let error = validate_and_save_provider_profile_impl(
        save_request("synthetic-create-cleanup-failure"),
        database.pool(),
        store,
        &runtime,
    )
    .await
    .unwrap_err();
    assert_eq!(error.code, AppErrorCode::CredentialStoreError);
    let dto = AppErrorDto::from(error);
    assert!(dto.diagnostic_id.is_some());
    let json = serde_json::to_string(&dto).unwrap();
    assert!(!json.contains("synthetic-create-cleanup-failure"));
    assert!(!json.contains("cleanup failed"));
}

#[tokio::test]
async fn replace_validates_before_snapshot_and_write() {
    let (_temporary, database) = test_database();
    let profile_id = Uuid::new_v4();
    insert_profile(database.pool(), profile_id, "openai", "gpt-5.6", 32_000).await;
    let store = Arc::new(ScriptedStore::with_failures(1, 0, 0));
    store.seed(
        &providers::credential_key(profile_id),
        "synthetic-original-credential",
    );
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models/gpt-5.6"))
        .respond_with(
            ResponseTemplate::new(401)
                .set_body_json(serde_json::json!({"error":{"code":"invalid_api_key"}})),
        )
        .mount(&server)
        .await;
    let runtime = test_runtime(store.clone(), &server);

    let error = replace_provider_profile_credential_impl(
        profile_id,
        SecretString::from("synthetic-rejected-replacement"),
        database.pool(),
        store.clone(),
        &runtime,
    )
    .await
    .unwrap_err();
    assert_eq!(error.code, AppErrorCode::InvalidApiKey);
    assert_eq!(store.get_calls.load(Ordering::SeqCst), 0);
    assert_eq!(store.set_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn replace_faults_before_write_or_restores_after_database_failure() {
    let (_temporary, database) = test_database();
    let profile_id = Uuid::new_v4();
    insert_profile(database.pool(), profile_id, "openai", "gpt-5.6", 32_000).await;
    let key = providers::credential_key(profile_id);
    let store = Arc::new(ScriptedStore::with_failures(0, 1, 0));
    store.seed(&key, "synthetic-original-before-write-failure");
    let server = validation_server().await;
    let runtime = test_runtime(store.clone(), &server);
    let write_error = replace_provider_profile_credential_impl(
        profile_id,
        SecretString::from("synthetic-write-failure"),
        database.pool(),
        store.clone(),
        &runtime,
    )
    .await
    .unwrap_err();
    assert_eq!(write_error.code, AppErrorCode::CredentialStoreError);
    assert_eq!(
        store.exposed_value(&key),
        Some("synthetic-original-before-write-failure".to_owned())
    );

    let (_temporary, database) = test_database();
    let profile_id = Uuid::new_v4();
    insert_profile(database.pool(), profile_id, "openai", "gpt-5.6", 32_000).await;
    install_update_failure(database.pool()).await;
    let key = providers::credential_key(profile_id);
    let store = Arc::new(ScriptedStore::default());
    store.seed(&key, "synthetic-original-restored");
    let server = validation_server().await;
    let runtime = test_runtime(store.clone(), &server);
    let database_error = replace_provider_profile_credential_impl(
        profile_id,
        SecretString::from("synthetic-new-rolled-back"),
        database.pool(),
        store.clone(),
        &runtime,
    )
    .await
    .unwrap_err();
    assert_eq!(database_error.code, AppErrorCode::DatabaseError);
    assert_eq!(
        store.exposed_value(&key),
        Some("synthetic-original-restored".to_owned())
    );
    assert_eq!(profile_context(database.pool(), profile_id).await, 32_000);
}

#[tokio::test]
async fn replace_restoration_failure_exposes_only_a_diagnostic_id() {
    let (_temporary, database) = test_database();
    let profile_id = Uuid::new_v4();
    insert_profile(database.pool(), profile_id, "openai", "gpt-5.6", 32_000).await;
    install_update_failure(database.pool()).await;
    let key = providers::credential_key(profile_id);
    let store = Arc::new(ScriptedStore::with_failures(0, 2, 0));
    store.seed(&key, "synthetic-old-restore-failure");
    let server = validation_server().await;
    let runtime = test_runtime(store.clone(), &server);

    let error = replace_provider_profile_credential_impl(
        profile_id,
        SecretString::from("synthetic-new-restore-failure"),
        database.pool(),
        store,
        &runtime,
    )
    .await
    .unwrap_err();
    assert_eq!(error.code, AppErrorCode::CredentialStoreError);
    let dto = AppErrorDto::from(error);
    assert!(dto.diagnostic_id.is_some());
    let json = serde_json::to_string(&dto).unwrap();
    assert!(!json.contains("synthetic-old-restore-failure"));
    assert!(!json.contains("synthetic-new-restore-failure"));
    assert!(!json.contains("restoration failed"));
}

#[tokio::test]
async fn cancellation_finishes_the_spawned_create_compensation_boundary() {
    let (_temporary, database) = test_database();
    let reached = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let store = Arc::new(ScriptedStore::with_set_barrier(
        1,
        reached.clone(),
        release.clone(),
    ));
    let server = validation_server().await;
    let runtime = Arc::new(test_runtime(store.clone(), &server));
    let pool = database.pool().clone();
    let task_store = store.clone();
    let task_runtime = runtime.clone();
    let task = tokio::spawn(async move {
        validate_and_save_provider_profile_impl(
            save_request("synthetic-cancelled-caller"),
            &pool,
            task_store,
            &task_runtime,
        )
        .await
    });
    reached.notified().await;
    task.abort();
    release.notify_one();

    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if profile_count(database.pool()).await == 1 {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(store.entry_count(), 1);
}

#[tokio::test]
async fn concurrent_duplicate_submission_conflicts_without_a_second_validation() {
    let (_temporary, database) = test_database();
    let reached = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let store = Arc::new(ScriptedStore::with_set_barrier(
        1,
        reached.clone(),
        release.clone(),
    ));
    let server = validation_server().await;
    let runtime = Arc::new(test_runtime(store.clone(), &server));
    let pool = database.pool().clone();
    let first_store = store.clone();
    let first_runtime = runtime.clone();
    let first = tokio::spawn(async move {
        validate_and_save_provider_profile_impl(
            save_request("synthetic-first-submission"),
            &pool,
            first_store,
            &first_runtime,
        )
        .await
    });
    reached.notified().await;

    let conflict = validate_and_save_provider_profile_impl(
        save_request("synthetic-duplicate-submission"),
        database.pool(),
        store,
        &runtime,
    )
    .await
    .unwrap_err();
    assert_eq!(conflict.code, AppErrorCode::RequestConflict);
    release.notify_one();
    first.await.unwrap().unwrap();
    assert_eq!(profile_count(database.pool()).await, 1);
}

#[tokio::test]
async fn keyring_unavailable_and_worker_panic_are_safe_and_leave_no_row() {
    let (_temporary, database) = test_database();
    let store = Arc::new(ScriptedStore::with_failures(0, 1, 0));
    let server = validation_server().await;
    let runtime = test_runtime(store.clone(), &server);
    let unavailable = validate_and_save_provider_profile_impl(
        save_request("synthetic-unavailable-store"),
        database.pool(),
        store,
        &runtime,
    )
    .await
    .unwrap_err();
    assert_eq!(unavailable.code, AppErrorCode::CredentialStoreError);
    assert_eq!(profile_count(database.pool()).await, 0);

    let (_temporary, database) = test_database();
    let store = Arc::new(ScriptedStore::with_set_panic(1));
    let server = validation_server().await;
    let runtime = test_runtime(store.clone(), &server);
    let process_error = validate_and_save_provider_profile_impl(
        save_request("synthetic-worker-process-error"),
        database.pool(),
        store,
        &runtime,
    )
    .await
    .unwrap_err();
    assert_eq!(process_error.code, AppErrorCode::DatabaseError);
    let dto = AppErrorDto::from(process_error);
    assert!(dto.diagnostic_id.is_some());
    assert_eq!(profile_count(database.pool()).await, 0);
    let json = serde_json::to_string(&dto).unwrap();
    assert!(!json.contains("synthetic-worker-process-error"));
}

fn test_runtime(store: Arc<ScriptedStore>, server: &MockServer) -> ProviderRuntime {
    ProviderRuntime::new_for_test(
        store,
        ProviderCapabilityRegistry::load_embedded().unwrap(),
        &server.uri(),
    )
    .unwrap()
}

fn test_chat_request(model: &str) -> UnifiedChatRequest {
    UnifiedChatRequest {
        model: model.to_owned(),
        system: "synthetic system".to_owned(),
        messages: vec![UnifiedMessage {
            role: UnifiedRole::User,
            content: "synthetic question".to_owned(),
        }],
        max_output_tokens: 32,
        expected_language: Some("en".to_owned()),
    }
}

fn test_structured_request(book_id: Uuid, model: &str) -> StructuredPageRequest {
    let limits = ImageLimits {
        max_images: 4,
        max_encoded_bytes_each: 4 * 1024 * 1024,
        max_total_encoded_bytes: 12 * 1024 * 1024,
        max_dimension_px: 4_096,
        max_decoded_pixels_each: 8_847_360,
    };
    let bytes = include_bytes!("../../../fixtures/source/vision/tiny-blue.png").to_vec();
    StructuredPageRequest {
        model: model.to_owned(),
        pages: vec![
            stage_vision_asset(book_id, Uuid::new_v4(), ImageMime::Png, 2, 2, bytes, limits)
                .unwrap(),
        ],
        schema_version: PAGE_ANALYSIS_SCHEMA_VERSION.to_owned(),
        max_output_bytes: 4_096,
    }
}

fn save_request(credential: &str) -> SaveProviderProfileRequest {
    SaveProviderProfileRequest::synthetic(
        ProviderKind::OpenAi,
        "Synthetic OpenAI profile",
        None,
        credential,
    )
}

async fn validation_server() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models/gpt-5.6"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"id":"gpt-5.6","object":"model"})),
        )
        .mount(&server)
        .await;
    server
}

fn test_database() -> (tempfile::TempDir, Database) {
    std::thread::spawn(|| {
        let temporary = tempfile::tempdir().unwrap();
        let database = Database::open(temporary.path().join("library.sqlite3")).unwrap();
        (temporary, database)
    })
    .join()
    .unwrap()
}

async fn insert_profile(
    pool: &sqlx::SqlitePool,
    profile_id: Uuid,
    provider_kind: &str,
    model_id: &str,
    context_window_tokens: i64,
) {
    let timestamp = chrono::Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO provider_profiles (id, provider_kind, display_name, model_id, context_window_tokens, is_active, created_at, updated_at, validated_at) VALUES (?, ?, 'Synthetic profile', ?, ?, 0, ?, ?, ?)",
    )
    .bind(profile_id.to_string())
    .bind(provider_kind)
    .bind(model_id)
    .bind(context_window_tokens)
    .bind(&timestamp)
    .bind(&timestamp)
    .bind(&timestamp)
    .execute(pool)
    .await
    .unwrap();
}

async fn install_insert_failure(pool: &sqlx::SqlitePool) {
    sqlx::query(
        "CREATE TRIGGER fail_provider_insert BEFORE INSERT ON provider_profiles BEGIN SELECT RAISE(ABORT, 'synthetic insert failure'); END",
    )
    .execute(pool)
    .await
    .unwrap();
}

async fn install_update_failure(pool: &sqlx::SqlitePool) {
    sqlx::query(
        "CREATE TRIGGER fail_provider_update BEFORE UPDATE ON provider_profiles BEGIN SELECT RAISE(ABORT, 'synthetic update failure'); END",
    )
    .execute(pool)
    .await
    .unwrap();
}

async fn profile_count(pool: &sqlx::SqlitePool) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM provider_profiles")
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn profile_context(pool: &sqlx::SqlitePool, profile_id: Uuid) -> i64 {
    sqlx::query_scalar("SELECT context_window_tokens FROM provider_profiles WHERE id = ?")
        .bind(profile_id.to_string())
        .fetch_one(pool)
        .await
        .unwrap()
}
