use secrecy::SecretString;
use textbooklens_lib::{
    ai::multimodal::stage_vision_asset,
    domain::{
        ImageLimits, ImageMime, ProviderKind, RemoteCleanupHandle, StructuredPageRequest,
        UnifiedChatRequest, UnifiedMessage, UnifiedRole, UnifiedVisionRequest,
    },
    errors::{AppError, AppErrorDto, redact},
};
use uuid::Uuid;

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

#[test]
fn multimodal_debug_and_error_dto_omit_raw_base64_schema_and_cleanup_sentinels() {
    let raw_image_sentinel = "fixture-raw-image-sentinel";
    let base64_sentinel = "Zml4dHVyZS1iYXNlNjQtc2VudGluZWw=";
    let schema_prompt_sentinel = "fixture-schema-prompt-sentinel";
    let cleanup_sentinel = "fixture-opaque-cleanup-sentinel";
    let model_sentinel = "fixture-model-sentinel";
    let limits = ImageLimits {
        max_images: 2,
        max_encoded_bytes_each: 1_024,
        max_total_encoded_bytes: 2_048,
        max_dimension_px: 4_096,
        max_decoded_pixels_each: 8_847_360,
    };
    let book_id = Uuid::new_v4();

    let vision = UnifiedVisionRequest {
        text: UnifiedChatRequest {
            model: model_sentinel.to_owned(),
            system: schema_prompt_sentinel.to_owned(),
            messages: vec![UnifiedMessage {
                role: UnifiedRole::User,
                content: base64_sentinel.to_owned(),
            }],
            max_output_tokens: 4_096,
            expected_language: None,
        },
        images: vec![asset_with_suffix(book_id, raw_image_sentinel, limits)],
    };
    let structured = StructuredPageRequest {
        model: model_sentinel.to_owned(),
        pages: vec![asset_with_suffix(book_id, "second-image", limits)],
        schema_version: "textbooklens.page-analysis.v1".to_owned(),
        max_output_bytes: 4_096,
    };
    let cleanup =
        RemoteCleanupHandle::new(ProviderKind::OpenAi, SecretString::from(cleanup_sentinel));

    let debug = format!("{vision:?} {structured:?} {cleanup:?}");
    for sentinel in [
        raw_image_sentinel,
        base64_sentinel,
        schema_prompt_sentinel,
        cleanup_sentinel,
        model_sentinel,
    ] {
        assert!(!debug.contains(sentinel));
    }

    let error_json = serde_json::to_string(&AppErrorDto::from(
        AppError::unsupported_provider_capability(),
    ))
    .unwrap();
    assert!(error_json.contains("UNSUPPORTED_PROVIDER_CAPABILITY"));
    for sentinel in [
        raw_image_sentinel,
        base64_sentinel,
        schema_prompt_sentinel,
        cleanup_sentinel,
        model_sentinel,
    ] {
        assert!(!error_json.contains(sentinel));
    }
}

fn asset_with_suffix(
    book_id: Uuid,
    suffix: &str,
    limits: ImageLimits,
) -> textbooklens_lib::domain::VisionAsset {
    let mut bytes = b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0dIHDR".to_vec();
    bytes.extend_from_slice(&10_u32.to_be_bytes());
    bytes.extend_from_slice(&10_u32.to_be_bytes());
    bytes.extend_from_slice(suffix.as_bytes());
    stage_vision_asset(
        book_id,
        Uuid::new_v4(),
        ImageMime::Png,
        10,
        10,
        bytes,
        limits,
    )
    .unwrap()
}
