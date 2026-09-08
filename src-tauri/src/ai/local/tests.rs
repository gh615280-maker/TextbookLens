use super::*;
use crate::{
    ai::{registry::ProviderCapabilityRegistry, runtime::ProviderRuntime},
    credentials::MemoryCredentialStore,
    db::{Database, providers},
    domain::{AiOperation, CredentialStatus, UnifiedMessage, UnifiedStreamEvent},
};
use futures_util::StreamExt;
use serde_json::json;
use std::sync::Arc;
use tempfile::TempDir;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{body_partial_json, method, path},
};

fn info() -> Value {
    json!({"capabilities":["completion"],"model_info":{"synthetic.context_length":32768}})
}
fn request() -> UnifiedChatRequest {
    UnifiedChatRequest {
        model: "synthetic:small".to_owned(),
        system: "Synthetic teaching instruction".to_owned(),
        messages: vec![UnifiedMessage {
            role: UnifiedRole::User,
            content: "What is 2 + 2?".to_owned(),
        }],
        max_output_tokens: 512,
        expected_language: None,
    }
}

#[tokio::test]
async fn local_vision_transmits_an_image_to_a_verified_local_vision_model() {
    use crate::domain::{ImageLimits, ImageMime, UnifiedVisionRequest};
    let server = MockServer::start().await;
    Mock::given(path("/api/tags"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"models":[]})))
        .mount(&server)
        .await;
    Mock::given(path("/api/show"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "capabilities":["completion","vision"],"model_info":{}
        })))
        .mount(&server)
        .await;
    Mock::given(path("/api/chat"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            "{\"message\":{\"content\":\"blue\"},\"done\":true,\"done_reason\":\"stop\"}\n",
        ))
        .expect(1)
        .mount(&server)
        .await;
    let bytes = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/source/vision/tiny-blue.png"
    ))
    .to_vec();
    let asset = crate::ai::multimodal::stage_vision_asset(
        uuid::Uuid::new_v4(),
        uuid::Uuid::new_v4(),
        ImageMime::Png,
        u32::from_be_bytes(bytes[16..20].try_into().unwrap()),
        u32::from_be_bytes(bytes[20..24].try_into().unwrap()),
        bytes,
        ImageLimits {
            max_images: 1,
            max_encoded_bytes_each: 1_000_000,
            max_total_encoded_bytes: 1_000_000,
            max_dimension_px: 2048,
            max_decoded_pixels_each: 4_194_304,
        },
    )
    .unwrap();
    let provider = LocalProvider::new(ProviderKind::Ollama, server.address().port(), 8192).unwrap();
    let stream = provider
        .stream_vision(
            &SecretString::default(),
            UnifiedVisionRequest {
                text: request(),
                images: vec![asset],
            },
            CancellationToken::new(),
        )
        .await;
    assert!(
        stream.is_ok(),
        "installed local vision model should accept an image"
    );
    let events = stream.unwrap().collect::<Vec<_>>().await;
    assert!(matches!(
        events.last(),
        Some(Ok(UnifiedStreamEvent::Completed))
    ));
    let requests = server.received_requests().await.unwrap();
    let sent = requests
        .iter()
        .find(|r| r.url.path() == "/api/chat")
        .unwrap();
    let body: Value = serde_json::from_slice(&sent.body).unwrap();
    assert_eq!(
        body["messages"][1]["images"][0],
        "iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAIAAAD91JpzAAAACXBIWXMAAAABAAAAAQBPJcTWAAAAEElEQVR4nGMwTDkJRAwQCgAj9gV5wT7pWAAAAABJRU5ErkJggg=="
    );
    assert_eq!(body["messages"][1]["content"], "What is 2 + 2?");
    assert!(!sent.headers.contains_key("authorization"));
}

#[tokio::test]
async fn local_vision_rechecks_capabilities_and_never_sends_images_to_text_or_remote_models() {
    for metadata in [
        info(),
        json!({"capabilities":["completion","vision"],"model_info":{},"remote_host":"https://example.invalid"}),
    ] {
        let server = MockServer::start().await;
        Mock::given(path("/api/tags"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"models":[]})))
            .mount(&server)
            .await;
        Mock::given(path("/api/show"))
            .respond_with(ResponseTemplate::new(200).set_body_json(metadata))
            .mount(&server)
            .await;
        Mock::given(path("/api/chat"))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&server)
            .await;
        let bytes = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../fixtures/source/vision/tiny-blue.png"
        ))
        .to_vec();
        let image = crate::ai::multimodal::stage_vision_asset(
            uuid::Uuid::new_v4(),
            uuid::Uuid::new_v4(),
            crate::domain::ImageMime::Png,
            2,
            2,
            bytes,
            LOCAL_IMAGE_LIMITS,
        )
        .unwrap();
        let provider =
            LocalProvider::new(ProviderKind::Ollama, server.address().port(), 8192).unwrap();
        assert!(
            provider
                .stream_vision(
                    &SecretString::default(),
                    crate::domain::UnifiedVisionRequest {
                        text: request(),
                        images: vec![image]
                    },
                    CancellationToken::new()
                )
                .await
                .is_err()
        );
    }
}

#[test]
fn local_vision_capability_and_separate_defaults_survive_restart_and_downgrade() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("vision.sqlite3");
    let database = Database::open(&path).unwrap();
    let registry = ProviderCapabilityRegistry::load_embedded().unwrap();
    let (vision_id, text_id) = tauri::async_runtime::block_on(async {
        let guard = providers::try_provider_mutation(database.pool())
            .await
            .unwrap();
        let models = [
            LocalModel {
                id: "synthetic:vision".into(),
                context: 16384,
                vision: true,
                size: 1,
                loaded: false,
            },
            LocalModel {
                id: "synthetic:text".into(),
                context: 8192,
                vision: false,
                size: 1,
                loaded: false,
            },
        ];
        let profiles =
            providers::save_local_models(database.pool(), &guard, ProviderKind::Ollama, 9, &models)
                .await
                .unwrap();
        providers::select_local_default(database.pool(), &guard, profiles[1].id)
            .await
            .unwrap();
        drop(guard);
        providers::set_default_provider_profile(
            database.pool(),
            &registry,
            AiOperation::VisionLearning,
            profiles[0].id,
        )
        .await
        .unwrap();
        assert!(
            providers::set_default_provider_profile(
                database.pool(),
                &registry,
                AiOperation::VisionLearning,
                profiles[1].id
            )
            .await
            .is_err()
        );
        let runtime =
            ProviderRuntime::new(Arc::new(MemoryCredentialStore::default()), registry.clone());
        assert!(
            runtime
                .load(database.pool(), profiles[0].id, AiOperation::VisionLearning)
                .await
                .is_ok()
        );
        assert!(
            runtime
                .load(
                    database.pool(),
                    profiles[0].id,
                    AiOperation::StructuredPageAnalysis
                )
                .await
                .is_err()
        );
        database.pool().close().await;
        (profiles[0].id, profiles[1].id)
    });
    let reopened = Database::open(&path).unwrap();
    tauri::async_runtime::block_on(async {
        let settings = crate::db::settings::get_app_settings(reopened.pool())
            .await
            .unwrap();
        assert_eq!(settings.default_learning_profile_id, Some(text_id));
        assert_eq!(settings.default_vision_profile_id, Some(vision_id));
        let public = crate::db::local_capabilities::public_registry(reopened.pool(), &registry)
            .await
            .unwrap();
        let provider = public
            .providers
            .iter()
            .find(|p| p.kind == ProviderKind::Ollama)
            .unwrap();
        assert_eq!(
            provider
                .models
                .iter()
                .find(|m| m.id == "synthetic:vision")
                .unwrap()
                .image_input,
            crate::domain::CapabilitySupport::Supported
        );
        let guard = providers::try_provider_mutation(reopened.pool())
            .await
            .unwrap();
        providers::save_local_models(
            reopened.pool(),
            &guard,
            ProviderKind::Ollama,
            9,
            &[LocalModel {
                id: "synthetic:vision".into(),
                context: 8192,
                vision: false,
                size: 1,
                loaded: false,
            }],
        )
        .await
        .unwrap();
        drop(guard);
        assert!(
            providers::set_default_provider_profile(
                reopened.pool(),
                &registry,
                AiOperation::VisionLearning,
                vision_id
            )
            .await
            .is_err()
        );
        reopened.pool().close().await;
    });
}

#[test]
fn local_inventory_excludes_cloud_remote_and_embedding_models() {
    assert!(local_ollama_info(&info()));
    let mut remote = info();
    remote["remote_host"] = json!("https://ollama.com");
    assert!(!local_ollama_info(&remote));
    assert!(!local_ollama_info(
        &json!({"capabilities":["embedding"],"model_info":{}})
    ));
    let models = parse_lmstudio_models(&json!([
        {"type":"llm","modelKey":"synthetic-local","deviceIdentifier":null,"sizeBytes":100,"maxContextLength":4096},
        {"type":"llm","modelKey":"remote","deviceIdentifier":"remote-device"},
        {"type":"embedding","modelKey":"embedding","deviceIdentifier":null}
    ])).unwrap();
    assert_eq!(models.len(), 1);
    assert_eq!(models[0].id, "synthetic-local");
    assert_eq!(models[0].context, 4096);
}

#[tokio::test]
async fn ollama_preserves_the_model_prompt_and_emits_only_the_final_answer() {
    let server = MockServer::start().await;
    Mock::given(path("/api/tags"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"models":[]})))
        .mount(&server)
        .await;
    let mut metadata = info();
    metadata["model_info"]["general.architecture"] = json!("qwen3");
    metadata["template"] = json!("old template");
    Mock::given(path("/api/show"))
        .respond_with(ResponseTemplate::new(200).set_body_json(metadata))
        .mount(&server)
        .await;
    Mock::given(path("/api/chat")).and(body_partial_json(json!({"messages":[{"role":"system","content":"Synthetic teaching instruction"},{"role":"user","content":"What is 2 + 2?"}]})))
        .respond_with(ResponseTemplate::new(200).set_body_string("{\"message\":{\"thinking\":\"hidden synthetic reasoning\"},\"done\":false}\n{\"message\":{\"content\":\"4\"},\"done\":true,\"done_reason\":\"stop\"}\n")).mount(&server).await;
    let provider = LocalProvider::new(ProviderKind::Ollama, server.address().port(), 8192).unwrap();
    let events = provider
        .stream_chat(
            &SecretString::default(),
            request(),
            CancellationToken::new(),
        )
        .await
        .unwrap()
        .collect::<Vec<_>>()
        .await;
    assert!(matches!(
        events.last(),
        Some(Ok(UnifiedStreamEvent::Completed))
    ));
    assert_eq!(events.len(), 2);
    assert_eq!(
        events[0].as_ref().unwrap(),
        &UnifiedStreamEvent::TextDelta {
            text: "4".to_owned()
        }
    );
}

#[tokio::test]
async fn ollama_cancel_discards_queued_completion() {
    let server = MockServer::start().await;
    Mock::given(path("/stream"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            "{\"message\":{\"content\":\"4\"},\"done\":true,\"done_reason\":\"stop\"}\n",
        ))
        .mount(&server)
        .await;
    let cancel = CancellationToken::new();
    let response = LocalHttp::new(server.address().port())
        .unwrap()
        .request("/stream", None, &cancel, METADATA_TIMEOUT)
        .await
        .unwrap();
    let mut stream = http::ollama_stream(response, cancel.clone());
    assert!(matches!(
        stream.next().await,
        Some(Ok(UnifiedStreamEvent::TextDelta { .. }))
    ));
    cancel.cancel();
    assert!(matches!(stream.next().await,Some(Err(error)) if error.kind()==AiErrorKind::Cancelled));
    assert!(stream.next().await.is_none());
}

#[test]
fn local_profiles_survive_restart_deduplicate_and_answer_without_credentials() {
    let temp = TempDir::new().unwrap();
    let db_path = temp.path().join("local.sqlite3");
    let database = Database::open(&db_path).unwrap();
    tauri::async_runtime::block_on(async {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/tags"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"models":[]})))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/show"))
            .respond_with(ResponseTemplate::new(200).set_body_json(info()))
            .mount(&server)
            .await;
        Mock::given(method("POST")).and(path("/api/chat")).and(body_partial_json(json!({"think":false,"options":{"num_ctx":8192},"messages":[{"role":"system","content":"Synthetic teaching instruction"},{"role":"user","content":"What is 2 + 2?"}]})))
            .respond_with(ResponseTemplate::new(200).set_body_string("{\"message\":{\"content\":\"4\"},\"done\":false}\n{\"message\":{\"content\":\"\"},\"done\":true,\"done_reason\":\"stop\"}\n")).expect(1).mount(&server).await;
        let guard = providers::try_provider_mutation(database.pool())
            .await
            .unwrap();
        let model = LocalModel {
            id: "synthetic:small".to_owned(),
            context: 8192,
            size: 100,
            loaded: true,
            vision: false,
        };
        let profiles = providers::save_local_models(
            database.pool(),
            &guard,
            ProviderKind::Ollama,
            server.address().port(),
            std::slice::from_ref(&model),
        )
        .await
        .unwrap();
        let again = providers::save_local_models(
            database.pool(),
            &guard,
            ProviderKind::Ollama,
            server.address().port(),
            &[model],
        )
        .await
        .unwrap();
        assert_eq!(profiles[0].id, again[0].id);
        providers::select_local_default(database.pool(), &guard, profiles[0].id)
            .await
            .unwrap();
        drop(guard);
        let store = Arc::new(MemoryCredentialStore::default());
        let list = providers::list_provider_profiles(database.pool(), store.as_ref())
            .await
            .unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].credential_status, CredentialStatus::NotRequired);
        assert!(list[0].is_active);
        let runtime = ProviderRuntime::new(
            store.clone(),
            ProviderCapabilityRegistry::load_embedded().unwrap(),
        );
        let provider = runtime
            .load(database.pool(), profiles[0].id, AiOperation::TextLearning)
            .await
            .unwrap();
        assert!(
            runtime
                .load(database.pool(), profiles[0].id, AiOperation::VisionLearning)
                .await
                .is_err()
        );
        let events = provider
            .stream_text_learning(request(), CancellationToken::new())
            .await
            .unwrap()
            .collect::<Vec<_>>()
            .await;
        assert_eq!(events.len(), 2);
        assert_eq!(
            events[0].as_ref().unwrap(),
            &UnifiedStreamEvent::TextDelta {
                text: "4".to_owned()
            }
        );
        assert_eq!(events[1].as_ref().unwrap(), &UnifiedStreamEvent::Completed);
        database.pool().close().await;
    });
    let database = Database::open(&db_path).unwrap();
    tauri::async_runtime::block_on(async {
        let store = Arc::new(MemoryCredentialStore::default());
        let list = providers::list_provider_profiles(database.pool(), store.as_ref())
            .await
            .unwrap();
        assert_eq!(list.len(), 1);
        assert!(list[0].is_active);
        providers::delete_provider_profile(database.pool(), store.clone(), list[0].id)
            .await
            .unwrap();
        assert!(
            providers::list_provider_profiles(database.pool(), store.as_ref())
                .await
                .unwrap()
                .is_empty()
        );
        database.pool().close().await;
    });
}

#[tokio::test]
async fn local_http_never_follows_redirects_and_remote_models_never_receive_prompts() {
    let remote = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&remote)
        .await;
    let server = MockServer::start().await;
    Mock::given(path("/redirect"))
        .respond_with(ResponseTemplate::new(302).insert_header("Location", remote.uri()))
        .mount(&server)
        .await;
    let client = LocalHttp::new(server.address().port()).unwrap();
    assert!(
        client
            .json(
                "/redirect",
                None,
                &CancellationToken::new(),
                METADATA_TIMEOUT
            )
            .await
            .is_err()
    );
    Mock::given(path("/api/tags"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"models":[]})))
        .mount(&server)
        .await;
    let mut remote_info = info();
    remote_info["remote_model"] = json!("remote-model");
    Mock::given(path("/api/show"))
        .respond_with(ResponseTemplate::new(200).set_body_json(remote_info))
        .mount(&server)
        .await;
    Mock::given(path("/api/chat"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;
    let provider = LocalProvider::new(ProviderKind::Ollama, server.address().port(), 8192).unwrap();
    assert!(
        provider
            .stream_chat(
                &SecretString::default(),
                request(),
                CancellationToken::new()
            )
            .await
            .is_err()
    );
}

#[tokio::test]
async fn lmstudio_stream_requires_a_successful_terminal_event() {
    let server = MockServer::start().await;
    Mock::given(path("/stream")).respond_with(ResponseTemplate::new(200).set_body_string("data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"4\"},\"finish_reason\":null}]}\n\n")).mount(&server).await;
    let http = LocalHttp::new(server.address().port()).unwrap();
    let cancel = CancellationToken::new();
    let response = http
        .request("/stream", None, &cancel, METADATA_TIMEOUT)
        .await
        .unwrap();
    let events = http::lmstudio_stream(response, cancel)
        .collect::<Vec<_>>()
        .await;
    assert!(events[0].is_ok());
    assert!(events.last().unwrap().is_err());
}

#[test]
#[ignore = "Requires an installed local runtime and already downloaded model; uses only a temporary database and synthetic question"]
fn installed_local_runtime_offline_smoke() {
    let temp = TempDir::new().unwrap();
    let database = Database::open(temp.path().join("smoke.sqlite3")).unwrap();
    tauri::async_runtime::block_on(async {
        let result = connect_local_models(database.pool()).await.unwrap();
        let id = result
            .default_profile_id
            .expect("a local model must load and answer");
        let provider = ProviderRuntime::new(
            Arc::new(MemoryCredentialStore::default()),
            ProviderCapabilityRegistry::load_embedded().unwrap(),
        )
        .load(database.pool(), id, AiOperation::TextLearning)
        .await
        .unwrap();
        let mut request = request();
        request.model = provider.profile().model_id.clone();
        request.max_output_tokens =
            crate::ai::registry::local_output_tokens(provider.profile().context_window_tokens);
        let events = provider
            .stream_text_learning(request, CancellationToken::new())
            .await
            .unwrap()
            .collect::<Vec<_>>()
            .await;
        assert!(
            events.iter().all(Result::is_ok),
            "stream errors: {:?}",
            events
                .iter()
                .filter_map(|event| event.as_ref().err().map(|error| error.kind()))
                .collect::<Vec<_>>()
        );
        assert!(events.iter().any(|event| matches!(event,Ok(UnifiedStreamEvent::TextDelta{text}) if !text.trim().is_empty())));
        assert!(matches!(
            events.last(),
            Some(Ok(UnifiedStreamEvent::Completed))
        ));
        database.pool().close().await;
    });
}

#[test]
#[ignore = "Requires an unused test-specific OLLAMA_HOST port; the test runner must clean up the server it starts"]
fn installed_ollama_service_starts_offline() {
    tauri::async_runtime::block_on(async {
        let installation = Installation::detect(ProviderKind::Ollama, None);
        assert_ne!(
            installation.port, 11434,
            "use an isolated port, never the user's normal server"
        );
        let http = LocalHttp::new(installation.port).unwrap();
        let cancel = CancellationToken::new();
        assert!(
            http.json("/api/tags", None, &cancel, METADATA_TIMEOUT)
                .await
                .is_err()
        );
        installation.ensure_running(&cancel).await.unwrap();
        let list = http
            .json("/api/tags", None, &cancel, METADATA_TIMEOUT)
            .await
            .unwrap();
        assert!(
            list["models"]
                .as_array()
                .is_some_and(|models| !models.is_empty())
        );
    });
}
