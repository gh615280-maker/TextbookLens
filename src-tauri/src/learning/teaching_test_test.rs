use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use parking_lot::Mutex;
use secrecy::SecretString;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

use crate::{
    ai::{registry::ProviderCapabilityRegistry, runtime::ProviderRuntime},
    credentials::CredentialStore,
    db::{Database, providers},
    errors::{AppError, AppErrorDto, AppResult},
};

use super::teaching_test::{
    TeachingTestEvent, TeachingTestRequest, event_for_result, run_teaching_test,
};

#[derive(Default)]
struct TestCredentialStore(Mutex<HashMap<String, SecretString>>);

impl TestCredentialStore {
    fn seed(&self, key: String, credential: &str) {
        self.0.lock().insert(key, SecretString::from(credential));
    }
}

#[async_trait]
impl CredentialStore for TestCredentialStore {
    async fn set(&self, key: &str, value: SecretString) -> AppResult<()> {
        self.0.lock().insert(key.to_owned(), value);
        Ok(())
    }

    async fn get(&self, key: &str) -> AppResult<SecretString> {
        self.0
            .lock()
            .get(key)
            .cloned()
            .ok_or_else(|| AppError::credential_store("synthetic missing credential"))
    }

    async fn delete(&self, key: &str) -> AppResult<()> {
        self.0.lock().remove(key);
        Ok(())
    }
}

#[tokio::test]
async fn teaching_test_uses_only_a_captured_learning_profile_and_keeps_all_rows_unchanged() {
    let (_temporary, database) = test_database();
    let profile_id = Uuid::new_v4();
    seed_learning_profile(database.pool(), profile_id).await;
    let store = Arc::new(TestCredentialStore::default());
    store.seed(
        providers::credential_key(profile_id),
        "synthetic-teaching-key",
    );
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(success_stream(), "text/event-stream"),
        )
        .expect(1)
        .mount(&server)
        .await;
    let runtime = test_runtime(store, &server);
    let before = protected_row_counts(database.pool()).await;
    let request = test_request();
    let emitted = Arc::new(Mutex::new(Vec::new()));
    let sink = emitted.clone();

    run_teaching_test(
        database.pool(),
        &runtime,
        request.clone(),
        CancellationToken::new(),
        move |event| sink.lock().push(event),
    )
    .await
    .unwrap();

    let events = emitted.lock().clone();
    assert!(events.iter().any(|event| matches!(
        event,
        TeachingTestEvent::TextDelta { request_id, text }
            if *request_id == request.request_id && text == "synthetic answer"
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        TeachingTestEvent::Usage {
            input_tokens: Some(7),
            output_tokens: Some(3),
            ..
        }
    )));
    assert!(matches!(
        events.last(),
        Some(TeachingTestEvent::Completed { .. })
    ));
    assert_eq!(protected_row_counts(database.pool()).await, before);
}

#[tokio::test]
async fn teaching_test_cancellation_after_a_delta_wins_over_late_completion() {
    let (_temporary, database) = test_database();
    let profile_id = Uuid::new_v4();
    seed_learning_profile(database.pool(), profile_id).await;
    let store = Arc::new(TestCredentialStore::default());
    store.seed(
        providers::credential_key(profile_id),
        "synthetic-cancel-key",
    );
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(success_stream(), "text/event-stream"),
        )
        .mount(&server)
        .await;
    let runtime = test_runtime(store, &server);
    let cancellation = CancellationToken::new();
    let cancellation_for_delta = cancellation.clone();
    let request = test_request();
    let emitted = Arc::new(Mutex::new(Vec::new()));
    let sink = emitted.clone();
    let result = run_teaching_test(
        database.pool(),
        &runtime,
        request.clone(),
        cancellation,
        move |event| {
            if matches!(event, TeachingTestEvent::TextDelta { .. }) {
                cancellation_for_delta.cancel();
            }
            sink.lock().push(event);
        },
    )
    .await;

    let terminal = event_for_result(request.request_id, result);
    assert!(matches!(
        terminal,
        Some(TeachingTestEvent::Cancelled { .. })
    ));
    assert!(
        !emitted
            .lock()
            .iter()
            .any(|event| matches!(event, TeachingTestEvent::Completed { .. }))
    );
}

#[tokio::test]
async fn teaching_test_cancellation_before_a_delta_emits_no_output() {
    let (_temporary, database) = test_database();
    let store = Arc::new(TestCredentialStore::default());
    let server = MockServer::start().await;
    let runtime = test_runtime(store, &server);
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let request = test_request();
    let emitted = Arc::new(Mutex::new(Vec::new()));
    let sink = emitted.clone();
    let result = run_teaching_test(
        database.pool(),
        &runtime,
        request.clone(),
        cancellation,
        move |event| sink.lock().push(event),
    )
    .await;

    assert!(emitted.lock().is_empty());
    assert!(matches!(
        event_for_result(request.request_id, result),
        Some(TeachingTestEvent::Cancelled { .. })
    ));
}

#[tokio::test]
async fn teaching_test_error_events_and_diagnostics_never_contain_request_or_vendor_sentinels() {
    let (_temporary, database) = test_database();
    let profile_id = Uuid::new_v4();
    seed_learning_profile(database.pool(), profile_id).await;
    let credential_sentinel = "teaching-credential-sentinel";
    let draft_sentinel = "teaching-draft-sentinel";
    let question_sentinel = "teaching-question-sentinel";
    let vendor_sentinel = "teaching-vendor-sentinel";
    let store = Arc::new(TestCredentialStore::default());
    store.seed(providers::credential_key(profile_id), credential_sentinel);
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(ResponseTemplate::new(503).set_body_raw(vendor_sentinel, "text/plain"))
        .mount(&server)
        .await;
    let runtime = test_runtime(store, &server);
    let request = TeachingTestRequest {
        instruction: draft_sentinel.to_owned(),
        question: question_sentinel.to_owned(),
        ..test_request()
    };
    let error = run_teaching_test(
        database.pool(),
        &runtime,
        request.clone(),
        CancellationToken::new(),
        |_| {},
    )
    .await
    .unwrap_err();
    let error_debug = format!("{error:?}");
    let error_display = error.to_string();
    let event = event_for_result(request.request_id, Err(error)).unwrap();
    let safe = serde_json::to_string(&event).unwrap();
    for sentinel in [
        credential_sentinel,
        draft_sentinel,
        question_sentinel,
        vendor_sentinel,
    ] {
        assert!(!safe.contains(sentinel));
        assert!(!error_debug.contains(sentinel));
        assert!(!error_display.contains(sentinel));
    }
    let dto = match event {
        TeachingTestEvent::Error { code, .. } => AppErrorDto {
            code,
            message: "safe".to_owned(),
            next_step: "safe".to_owned(),
            diagnostic_id: None,
        },
        _ => panic!("provider failure must be an error event"),
    };
    let dto_json = serde_json::to_string(&dto).unwrap();
    assert!(!dto_json.contains(vendor_sentinel));
}

fn test_runtime(store: Arc<TestCredentialStore>, server: &MockServer) -> ProviderRuntime {
    ProviderRuntime::new_for_test(
        store,
        ProviderCapabilityRegistry::load_embedded().unwrap(),
        &server.uri(),
    )
    .unwrap()
}

fn test_request() -> TeachingTestRequest {
    TeachingTestRequest {
        session_id: Uuid::new_v4(),
        request_id: Uuid::new_v4(),
        instruction: "Use short visible steps.".to_owned(),
        question: "What is a synthetic concept?".to_owned(),
    }
}

fn success_stream() -> &'static str {
    "event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"synthetic answer\"}\n\nevent: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\",\"usage\":{\"input_tokens\":7,\"output_tokens\":3}}}\n\n"
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

async fn seed_learning_profile(pool: &sqlx::SqlitePool, profile_id: Uuid) {
    let timestamp = chrono::Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO provider_profiles (id, provider_kind, display_name, model_id, context_window_tokens, is_active, created_at, updated_at, validated_at) VALUES (?, 'openai', 'Synthetic profile', 'gpt-5.6', 32000, 1, ?, ?, ?)",
    )
    .bind(profile_id.to_string())
    .bind(&timestamp)
    .bind(&timestamp)
    .bind(&timestamp)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query("UPDATE app_settings SET default_learning_profile_id = ?, active_provider_profile_id = ? WHERE id = 1")
        .bind(profile_id.to_string())
        .bind(profile_id.to_string())
        .execute(pool)
        .await
        .unwrap();
}

async fn protected_row_counts(pool: &sqlx::SqlitePool) -> Vec<(&'static str, i64)> {
    vec![
        (
            "teaching_preferences",
            sqlx::query_scalar("SELECT COUNT(*) FROM teaching_preferences")
                .fetch_one(pool)
                .await
                .unwrap(),
        ),
        (
            "books",
            sqlx::query_scalar("SELECT COUNT(*) FROM books")
                .fetch_one(pool)
                .await
                .unwrap(),
        ),
        (
            "search_chunks",
            sqlx::query_scalar("SELECT COUNT(*) FROM search_chunks")
                .fetch_one(pool)
                .await
                .unwrap(),
        ),
        (
            "conversations",
            sqlx::query_scalar("SELECT COUNT(*) FROM conversations")
                .fetch_one(pool)
                .await
                .unwrap(),
        ),
        (
            "messages",
            sqlx::query_scalar("SELECT COUNT(*) FROM messages")
                .fetch_one(pool)
                .await
                .unwrap(),
        ),
        (
            "annotations",
            sqlx::query_scalar("SELECT COUNT(*) FROM annotations")
                .fetch_one(pool)
                .await
                .unwrap(),
        ),
    ]
}
