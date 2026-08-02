use textbooklens_lib::errors::{AppError, AppErrorDto, redact};

#[test]
fn dto_never_serializes_secret_or_provider_body() {
    let fake_key = ["sk", "live", "secret"].join("-");
    let err = AppError::invalid_api_key(&fake_key, "raw provider body");
    assert!(!err.diagnostic_detail().unwrap().contains(&fake_key));
    assert!(
        !err.diagnostic_detail()
            .unwrap()
            .contains("raw provider body")
    );
    let json = serde_json::to_string(&AppErrorDto::from(err)).unwrap();

    assert!(json.contains("INVALID_API_KEY"));
    assert!(!json.contains(&fake_key));
    assert!(!json.contains("raw provider body"));
}

#[test]
fn redact_filters_authorization_and_common_key_shapes() {
    let authorization = ["Authorization:", "Bearer", "abc123"].join(" ");
    let fake_key = ["sk", "test", "123"].join("-");
    let safe = redact(&format!("{authorization} {fake_key} x-goog-api-key: xyz"));

    assert_eq!(
        safe,
        "Authorization: [REDACTED] [REDACTED] x-goog-api-key: [REDACTED]"
    );
}
