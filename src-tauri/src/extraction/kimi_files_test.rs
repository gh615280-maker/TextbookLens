use secrecy::SecretString;
use tokio_util::sync::CancellationToken;
use wiremock::{
    Mock, ResponseTemplate,
    matchers::{header, method, path},
};

use super::kimi_files::{FileState, KimiFilesClient, MAX_FILE_BYTES};
use crate::errors::AppErrorCode;

const KEY: &str = "fixture-kimi-files-key";
const UPLOAD: &str = include_str!("../../../fixtures/providers/kimi/files-upload-ok.json");
const PROCESSING: &str = include_str!("../../../fixtures/providers/kimi/files-processing.json");
const READY: &str = include_str!("../../../fixtures/providers/kimi/files-ready.json");
const FAILED: &str = include_str!("../../../fixtures/providers/kimi/files-failed.json");
const CONTENT: &str = include_str!("../../../fixtures/providers/kimi/files-content.txt");

async fn client() -> (wiremock::MockServer, KimiFilesClient) {
    let server = wiremock::MockServer::start().await;
    let client = KimiFilesClient::new_for_test(&format!("{}/v1", server.uri())).unwrap();
    (server, client)
}

#[tokio::test]
async fn upload_is_one_bounded_multipart_file_extract_request() {
    let (server, client) = client().await;
    Mock::given(method("POST"))
        .and(path("/v1/files"))
        .and(header("authorization", format!("Bearer {KEY}")))
        .respond_with(ResponseTemplate::new(200).set_body_raw(UPLOAD, "application/json"))
        .expect(1)
        .mount(&server)
        .await;
    let uploaded = client
        .upload(
            &SecretString::from(KEY),
            b"%PDF-synthetic".to_vec(),
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(
        secrecy::ExposeSecret::expose_secret(uploaded.id()),
        "file_fixture_123"
    );
    let requests = server.received_requests().await.unwrap();
    let body = String::from_utf8_lossy(&requests[0].body);
    assert!(body.contains("file-extract"));
    assert!(body.contains("%PDF-synthetic"));
    assert!(!body.contains(KEY));
}

#[tokio::test]
async fn status_content_and_delete_follow_files_contract() {
    for (fixture, expected) in [
        (PROCESSING, FileState::Processing),
        (READY, FileState::Ready),
        (FAILED, FileState::Failed),
    ] {
        let (server, client) = client().await;
        Mock::given(method("GET"))
            .and(path("/v1/files/file_fixture_123"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(fixture, "application/json"))
            .expect(1)
            .mount(&server)
            .await;
        assert_eq!(
            client
                .status(
                    &SecretString::from(KEY),
                    &SecretString::from("file_fixture_123"),
                    CancellationToken::new()
                )
                .await
                .unwrap(),
            expected
        );
    }
    let (server, client) = client().await;
    Mock::given(method("GET"))
        .and(path("/v1/files/file_fixture_123/content"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(CONTENT, "text/plain"))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/v1/files/file_fixture_123"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    let id = SecretString::from("file_fixture_123");
    assert_eq!(
        client
            .content(&SecretString::from(KEY), &id, CancellationToken::new())
            .await
            .unwrap(),
        CONTENT
    );
    client
        .delete(&SecretString::from(KEY), &id, CancellationToken::new())
        .await
        .unwrap();
}

#[tokio::test]
async fn limits_cancel_and_http_failures_map_without_vendor_or_secret_leaks() {
    let (_server, files_client) = client().await;
    let oversized = files_client
        .upload(
            &SecretString::from(KEY),
            vec![0; MAX_FILE_BYTES + 1],
            CancellationToken::new(),
        )
        .await
        .unwrap_err();
    assert_eq!(oversized.code, AppErrorCode::InvalidInput);
    let cancel = CancellationToken::new();
    cancel.cancel();
    let cancelled = files_client
        .upload(&SecretString::from(KEY), b"pdf".to_vec(), cancel)
        .await
        .unwrap_err();
    assert_eq!(cancelled.code, AppErrorCode::ImportCancelled);
    for (status, code) in [
        (429, AppErrorCode::RateLimited),
        (503, AppErrorCode::ProviderUnavailable),
    ] {
        let (server, client) = client().await;
        Mock::given(method("POST"))
            .and(path("/v1/files"))
            .respond_with(ResponseTemplate::new(status).set_body_raw(
                r#"{"error":{"message":"vendor-sentinel"}}"#,
                "application/json",
            ))
            .expect(1)
            .mount(&server)
            .await;
        let error = client
            .upload(
                &SecretString::from(KEY),
                b"pdf".to_vec(),
                CancellationToken::new(),
            )
            .await
            .unwrap_err();
        assert_eq!(error.code, code);
        let safe = format!("{error:?} {error}");
        assert!(!safe.contains("vendor-sentinel"));
        assert!(!safe.contains(KEY));
    }

    let server = wiremock::MockServer::start().await;
    let client =
        KimiFilesClient::new_for_test_with_content_limit(&format!("{}/v1", server.uri()), 8)
            .unwrap();
    Mock::given(method("GET"))
        .and(path("/v1/files/file_fixture_123/content"))
        .respond_with(ResponseTemplate::new(200).set_body_raw("bounded fixture", "text/plain"))
        .expect(1)
        .mount(&server)
        .await;
    let error = client
        .content(
            &SecretString::from(KEY),
            &SecretString::from("file_fixture_123"),
            CancellationToken::new(),
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, AppErrorCode::ContextTooLarge);
}
