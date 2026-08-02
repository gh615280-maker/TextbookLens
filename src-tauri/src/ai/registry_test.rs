use super::registry::{ProviderCapabilityRegistry, RegistryError};
use crate::domain::ProviderKind;

#[test]
fn embedded_registry_has_exactly_the_supported_provider_kinds() {
    let registry = ProviderCapabilityRegistry::load_embedded().expect("embedded registry is valid");
    assert_eq!(registry.capabilities().len(), 5);
    assert_eq!(
        registry
            .capabilities()
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
    for capability in registry.capabilities() {
        assert!(
            capability
                .models
                .iter()
                .any(|model| model.id == capability.default_model)
        );
        assert!(capability.models.iter().all(
            |model| model.context_window_tokens > 0 && model.default_max_output_tokens >= 4096
        ));
    }
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
fn registry_rejects_unknown_fields_and_duplicate_ids() {
    let unknown_field = r#"{
      "providers": [],
      "proxy_url": "https://example.invalid"
    }"#;
    assert!(matches!(
        ProviderCapabilityRegistry::from_json(unknown_field),
        Err(RegistryError::Parse(_))
    ));

    let duplicate_provider = r#"{
      "providers": [
        {"kind":"openai","display_name":"OpenAI","default_model":"gpt-5.6","models":[{"id":"gpt-5.6","display_name":"GPT-5.6","context_window_tokens":1050000,"default_max_output_tokens":4096,"vendor_max_output_tokens":128000}]},
        {"kind":"openai","display_name":"OpenAI again","default_model":"gpt-5.6","models":[{"id":"gpt-5.6","display_name":"GPT-5.6","context_window_tokens":1050000,"default_max_output_tokens":4096,"vendor_max_output_tokens":128000}]},
        {"kind":"gemini","display_name":"Gemini","default_model":"gemini-3.6-flash","models":[{"id":"gemini-3.6-flash","display_name":"Gemini 3.6 Flash","context_window_tokens":1048576,"default_max_output_tokens":4096,"vendor_max_output_tokens":65536}]},
        {"kind":"anthropic","display_name":"Anthropic","default_model":"claude-sonnet-5","models":[{"id":"claude-sonnet-5","display_name":"Claude Sonnet 5","context_window_tokens":1000000,"default_max_output_tokens":4096,"vendor_max_output_tokens":128000}]},
        {"kind":"deepseek","display_name":"DeepSeek","default_model":"deepseek-v4-flash","models":[{"id":"deepseek-v4-flash","display_name":"DeepSeek V4 Flash","context_window_tokens":1000000,"default_max_output_tokens":4096,"vendor_max_output_tokens":393216}]},
        {"kind":"kimi","display_name":"Kimi","default_model":"kimi-k3","models":[{"id":"kimi-k3","display_name":"Kimi K3","context_window_tokens":1048576,"default_max_output_tokens":4096,"vendor_max_output_tokens":1048576}]}
      ]
    }"#;
    assert!(matches!(
        ProviderCapabilityRegistry::from_json(duplicate_provider),
        Err(RegistryError::Invalid(_))
    ));

    let first_model = r#"        {
          "id": "gpt-5.6",
          "display_name": "GPT-5.6",
          "context_window_tokens": 1050000,
          "default_max_output_tokens": 4096,
          "vendor_max_output_tokens": 128000
        }"#;
    let duplicate_model = include_str!("../../resources/provider-models.json").replacen(
        first_model,
        &format!("{first_model},\n{first_model}"),
        1,
    );
    assert!(matches!(
        ProviderCapabilityRegistry::from_json(&duplicate_model),
        Err(RegistryError::Invalid(_))
    ));
}
