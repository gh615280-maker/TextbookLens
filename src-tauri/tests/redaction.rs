use textbooklens_lib::errors::{AppError, AppErrorDto, redact};

#[test]
fn redaction_dto_never_serializes_secret_or_provider_body() {
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
fn redaction_filters_authorization_and_common_key_shapes() {
    let authorization = ["Authorization:", "Bearer", "abc123"].join(" ");
    let fake_key = ["sk", "test", "123"].join("-");
    let safe = redact(&format!("{authorization} {fake_key} x-goog-api-key: xyz"));

    assert_eq!(
        safe,
        "Authorization: [REDACTED] [REDACTED] x-goog-api-key: [REDACTED]"
    );
}

#[test]
fn redaction_filters_query_unicode_and_nested_structured_fields() {
    let unicode_secret = "测试凭据-秘密";
    let query_secret = "fixture-query-sentinel";
    let nested_secret = "fixture-nested-sentinel";
    let input = format!(
        r#"request_url=https://example.invalid/v1?key={query_secret}&mode=test {{"outer":{{"authorization":"Bearer {unicode_secret}","x-api-key":"{nested_secret}"}}}}"#
    );
    let safe = redact(&input);

    assert!(!safe.contains(unicode_secret));
    assert!(!safe.contains(query_secret));
    assert!(!safe.contains(nested_secret));
    assert!(safe.contains("mode=test"));
}
