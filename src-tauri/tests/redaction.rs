use textbooklens_lib::errors::{AppError, AppErrorDto, redact};

#[test]
fn dto_never_serializes_secret_or_provider_body() {
    let err = AppError::invalid_api_key("sk-live-secret", "raw provider body");
    assert!(!err.diagnostic_detail().unwrap().contains("sk-live-secret"));
    assert!(
        !err.diagnostic_detail()
            .unwrap()
            .contains("raw provider body")
    );
    let json = serde_json::to_string(&AppErrorDto::from(err)).unwrap();

    assert!(json.contains("INVALID_API_KEY"));
    assert!(!json.contains("sk-live-secret"));
    assert!(!json.contains("raw provider body"));
}

#[test]
fn redact_filters_authorization_and_common_key_shapes() {
    let safe = redact("Authorization: Bearer abc123 sk-test-123 x-goog-api-key: xyz");

    assert_eq!(
        safe,
        "Authorization: [REDACTED] [REDACTED] x-goog-api-key: [REDACTED]"
    );
}
