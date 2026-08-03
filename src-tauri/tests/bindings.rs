use std::collections::BTreeMap;

use textbooklens_lib::domain::{
    AiOperation, AnnotationDto, AppSettingsDto, BlockKind, BookSummary, CapabilitySupport,
    ConversationDto, CredentialStatus, DocumentLocator, ImageLimits, ImageMime, LearningEvent,
    LearningRequest, LocalTextQuality, NormalizedBookInput, NormalizedRect, OnboardingStateDto,
    OnboardingStep, PageAnalysisBlockKind, ProviderCapability, ProviderCapabilityRegistryDto,
    ProviderModelCapability, ProviderOperationConsent, ProviderOperationConsentCategory,
    ProviderOperationConsentDecision, ProviderPageAnalysis, ProviderProfileSummary,
    RemoteCleanupStatus, UiLanguage, UnifiedChatRequest, UnifiedMessage, UnifiedRole,
    UnifiedStreamEvent, UntrustedNormalizedRect, UntrustedPageAnalysis, UntrustedPageBlock,
    UntrustedTableCell, ValidationResult, VisionAssetMeta, stable_block_id, stable_section_id,
};
use ts_rs::{Config, TS};

#[test]
fn pdf_locator_is_internally_tagged_and_one_based() {
    let locator = DocumentLocator::pdf(1, 2, None).expect("one-based pages are valid");

    assert_eq!(
        serde_json::to_value(locator).unwrap(),
        serde_json::json!({
            "format": "pdf",
            "startPage": 1,
            "endPage": 2,
            "rectsByPage": null
        })
    );
}

#[test]
fn stable_block_id_uses_book_namespace() {
    let book = uuid::uuid!("4f9a2c86-0da8-4dd4-a255-39b4cff89c66");
    assert_eq!(
        stable_section_id(book, 3).to_string(),
        "70c92c3b-d44a-5345-a7eb-839e79c5b322"
    );
    assert_eq!(
        stable_block_id(book, 3, 7).to_string(),
        "2af8bb3b-d9be-5fb6-9246-86a57e8eec56"
    );
}

#[test]
fn block_kind_serializes_to_the_canonical_parser_contract() {
    let kinds = [
        BlockKind::Heading,
        BlockKind::Paragraph,
        BlockKind::List,
        BlockKind::Table,
        BlockKind::Caption,
        BlockKind::Equation,
    ];
    let serialized = kinds
        .into_iter()
        .map(|kind| serde_json::to_string(&kind).unwrap())
        .collect::<Vec<_>>();

    assert_eq!(
        serialized,
        [
            "\"heading\"",
            "\"paragraph\"",
            "\"list\"",
            "\"table\"",
            "\"caption\"",
            "\"equation\"",
        ]
    );
}

#[test]
fn ui_language_serializes_to_product_locale_labels() {
    let values = [UiLanguage::ZhCn, UiLanguage::ZhTw, UiLanguage::En]
        .into_iter()
        .map(|language| serde_json::to_value(language).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        values,
        [
            serde_json::json!("zh-CN"),
            serde_json::json!("zh-TW"),
            serde_json::json!("en"),
        ]
    );
}

#[test]
fn document_constructors_reject_invalid_values() {
    assert!(DocumentLocator::pdf(0, 1, None).is_err());
    assert!(DocumentLocator::pdf(2, 1, None).is_err());
    assert!(NormalizedRect::new(0.0, 0.0, 1.1, 0.5).is_err());
    assert!(DocumentLocator::docx(uuid::Uuid::new_v4(), 3, uuid::Uuid::new_v4(), 2).is_err());

    let rects = BTreeMap::from([(1, vec![NormalizedRect::new(0.0, 0.0, 0.5, 0.5).unwrap()])]);
    assert!(DocumentLocator::pdf(1, 1, Some(rects)).is_ok());
}

#[test]
fn export_bindings() {
    let output_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../src/lib/generated");
    std::fs::create_dir_all(&output_dir).unwrap();
    let config = Config::new().with_out_dir(output_dir);

    BookSummary::export_all(&config).unwrap();
    NormalizedBookInput::export_all(&config).unwrap();
    AnnotationDto::export_all(&config).unwrap();
    ConversationDto::export_all(&config).unwrap();
    LearningRequest::export_all(&config).unwrap();
    LearningEvent::export_all(&config).unwrap();
    CapabilitySupport::export_all(&config).unwrap();
    AiOperation::export_all(&config).unwrap();
    ImageLimits::export_all(&config).unwrap();
    ProviderModelCapability::export_all(&config).unwrap();
    ProviderCapability::export_all(&config).unwrap();
    ProviderCapabilityRegistryDto::export_all(&config).unwrap();
    CredentialStatus::export_all(&config).unwrap();
    ProviderOperationConsentCategory::export_all(&config).unwrap();
    ProviderOperationConsentDecision::export_all(&config).unwrap();
    ProviderOperationConsent::export_all(&config).unwrap();
    ProviderProfileSummary::export_all(&config).unwrap();
    ValidationResult::export_all(&config).unwrap();
    UnifiedRole::export_all(&config).unwrap();
    UnifiedMessage::export_all(&config).unwrap();
    UnifiedChatRequest::export_all(&config).unwrap();
    UnifiedStreamEvent::export_all(&config).unwrap();
    ImageMime::export_all(&config).unwrap();
    VisionAssetMeta::export_all(&config).unwrap();
    PageAnalysisBlockKind::export_all(&config).unwrap();
    UntrustedNormalizedRect::export_all(&config).unwrap();
    UntrustedTableCell::export_all(&config).unwrap();
    UntrustedPageBlock::export_all(&config).unwrap();
    UntrustedPageAnalysis::export_all(&config).unwrap();
    ProviderPageAnalysis::export_all(&config).unwrap();
    RemoteCleanupStatus::export_all(&config).unwrap();
    AppSettingsDto::export_all(&config).unwrap();
    OnboardingStep::export_all(&config).unwrap();
    LocalTextQuality::export_all(&config).unwrap();
    OnboardingStateDto::export_all(&config).unwrap();
    UiLanguage::export_all(&config).unwrap();
}
