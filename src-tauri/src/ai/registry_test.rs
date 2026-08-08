use chrono::Utc;
use serde_json::{Value, json};
use uuid::Uuid;

use super::registry::{ProviderCapabilityRegistry, RegistryError};
use crate::domain::{
    AiOperation, CapabilitySupport, CredentialStatus, KimiApiRegion, ProviderKind,
    ProviderProfileSummary,
};

const EMBEDDED_REGISTRY: &str = include_str!("../../resources/provider-models.json");

#[test]
fn embedded_registry_exposes_versioned_verified_capability_truth() {
    let registry = ProviderCapabilityRegistry::load_embedded().expect("embedded registry is valid");
    let public = registry.public_registry();

    assert_eq!(public.schema_version, 1);
    assert_eq!(public.providers.len(), 5);
    assert_eq!(
        public
            .providers
            .iter()
            .map(|capability| capability.kind.clone())
            .collect::<Vec<_>>(),
        vec![
            ProviderKind::OpenAi,
            ProviderKind::Gemini,
            ProviderKind::Anthropic,
            ProviderKind::DeepSeek,
            ProviderKind::Kimi,
        ]
    );
    for capability in &public.providers {
        assert!(
            capability
                .models
                .iter()
                .any(|model| model.id == capability.default_model)
        );
        assert!(capability.models.iter().all(|model| {
            model.context_window_tokens > 0
                && model.default_max_output_tokens >= 4_096
                && model.last_verified == "2026-08-03"
                && model.text_chat == CapabilitySupport::Supported
        }));
    }

    let deepseek = &public.providers[3].models[0];
    assert_eq!(deepseek.image_input, CapabilitySupport::Unsupported);
    assert_eq!(deepseek.pdf_input, CapabilitySupport::Unsupported);
    assert_eq!(
        deepseek.strict_structured_output,
        CapabilitySupport::Supported
    );
    assert!(deepseek.image_limits.is_none());

    let kimi = &public.providers[4].models[0];
    assert_eq!(kimi.image_input, CapabilitySupport::Supported);
    assert_eq!(kimi.pdf_input, CapabilitySupport::Unsupported);
    assert_eq!(kimi.native_pdf_input, CapabilitySupport::Unsupported);
    let kimi_provider = registry
        .capabilities()
        .iter()
        .find(|p| p.kind == ProviderKind::Kimi)
        .unwrap();
    assert!(kimi_provider.file_capabilities.file_extraction);
    assert!(kimi_provider.file_capabilities.file_ocr);
    assert_eq!(
        kimi_provider.file_capabilities.max_file_bytes,
        Some(100 * 1024 * 1024)
    );
    assert!(kimi.image_limits.is_some());

    assert_eq!(
        registry.resolve_context_window(&ProviderKind::OpenAi, "gpt-5.6", None),
        1_050_000
    );
    assert_eq!(
        registry.resolve_context_window(&ProviderKind::OpenAi, "a-user-model", None),
        32_000
    );
    assert_eq!(
        registry.resolve_context_window(&ProviderKind::OpenAi, "a-user-model", Some(96_000)),
        96_000
    );
    assert_eq!(
        registry.resolve_context_window(&ProviderKind::OpenAi, "a-user-model", Some(0)),
        32_000
    );
}

#[test]
fn registry_strictly_rejects_incomplete_or_inconsistent_records() {
    let mut unknown_field = embedded_value();
    unknown_field["proxy_url"] = json!("https://example.invalid");
    assert_parse_error(unknown_field);

    let mut missing_last_verified = embedded_value();
    first_model_mut(&mut missing_last_verified)
        .as_object_mut()
        .unwrap()
        .remove("last_verified");
    assert_parse_error(missing_last_verified);

    let mut missing_image_limits = embedded_value();
    first_model_mut(&mut missing_image_limits)["image_limits"] = Value::Null;
    assert_invalid(missing_image_limits);

    let mut illegal_image_limit = embedded_value();
    first_model_mut(&mut illegal_image_limit)["image_limits"]["max_images"] = json!(0);
    assert_invalid(illegal_image_limit);

    let mut default_model_mismatch = embedded_value();
    default_model_mismatch["providers"][0]["default_model"] = json!("not-registered");
    assert_invalid(default_model_mismatch);

    let mut duplicate_model = embedded_value();
    let duplicate = duplicate_model["providers"][0]["models"][0].clone();
    duplicate_model["providers"][0]["models"]
        .as_array_mut()
        .unwrap()
        .push(duplicate);
    assert_invalid(duplicate_model);

    let mut wrong_version = embedded_value();
    wrong_version["schema_version"] = json!(2);
    assert_invalid(wrong_version);
}

#[test]
fn operation_resolution_denies_unverified_visual_and_schema_fallbacks() {
    let registry = ProviderCapabilityRegistry::load_embedded().expect("embedded registry is valid");
    let openai = profile(ProviderKind::OpenAi, "gpt-5.6", true);
    let deepseek = profile(ProviderKind::DeepSeek, "deepseek-v4-flash", true);
    let custom_verified = profile(ProviderKind::OpenAi, "custom-text-model", true);
    let custom_unverified = profile(ProviderKind::OpenAi, "unverified-model", false);
    let profiles = vec![
        openai.clone(),
        deepseek.clone(),
        custom_verified.clone(),
        custom_unverified,
    ];

    assert_eq!(
        registry
            .resolve_operation(AiOperation::TextLearning, &profiles)
            .iter()
            .map(|profile| profile.id)
            .collect::<Vec<_>>(),
        vec![openai.id, deepseek.id, custom_verified.id]
    );
    assert_eq!(
        registry.resolve_operation(AiOperation::VisionLearning, &profiles),
        vec![openai.clone()]
    );
    assert_eq!(
        registry.resolve_operation(AiOperation::StructuredPageAnalysis, &profiles),
        vec![openai]
    );
    assert_eq!(
        registry.operation_support(
            &ProviderKind::OpenAi,
            "custom-text-model",
            AiOperation::VisionLearning,
        ),
        CapabilitySupport::Unknown
    );
    assert_eq!(
        registry.operation_support(
            &ProviderKind::DeepSeek,
            "deepseek-v4-flash",
            AiOperation::StructuredPageAnalysis,
        ),
        CapabilitySupport::Unsupported
    );
}

#[test]
fn capability_and_operation_discriminants_are_stable_snake_case() {
    assert_eq!(
        serde_json::to_value(CapabilitySupport::Supported).unwrap(),
        json!("supported")
    );
    assert_eq!(
        serde_json::to_value(CapabilitySupport::Unsupported).unwrap(),
        json!("unsupported")
    );
    assert_eq!(
        serde_json::to_value(CapabilitySupport::Unknown).unwrap(),
        json!("unknown")
    );
    assert_eq!(
        serde_json::to_value(AiOperation::StructuredPageAnalysis).unwrap(),
        json!("structured_page_analysis")
    );
}

fn embedded_value() -> Value {
    serde_json::from_str(EMBEDDED_REGISTRY).unwrap()
}

fn first_model_mut(document: &mut Value) -> &mut Value {
    &mut document["providers"][0]["models"][0]
}

fn assert_parse_error(document: Value) {
    let json = serde_json::to_string(&document).unwrap();
    assert!(matches!(
        ProviderCapabilityRegistry::from_json(&json),
        Err(RegistryError::Parse(_))
    ));
}

fn assert_invalid(document: Value) {
    let json = serde_json::to_string(&document).unwrap();
    assert!(matches!(
        ProviderCapabilityRegistry::from_json(&json),
        Err(RegistryError::Invalid(_))
    ));
}

fn profile(kind: ProviderKind, model_id: &str, validated: bool) -> ProviderProfileSummary {
    let kimi_api_region = (kind == ProviderKind::Kimi).then_some(KimiApiRegion::Cn);
    ProviderProfileSummary {
        id: Uuid::new_v4(),
        kind,
        display_name: model_id.to_owned(),
        model_id: model_id.to_owned(),
        context_window_tokens: 32_000,
        is_active: false,
        credential_status: CredentialStatus::Available,
        validated_at: validated.then(Utc::now),
        kimi_api_region,
    }
}
