use std::collections::BTreeMap;

use textbooklens_lib::domain::{
    AiOperation, AnnotationDto, AppSettingsDto, BlockKind, BookIndexAggregateStatus, BookSummary,
    CapabilitySupport, ContentSource, ConversationDto, CredentialStatus, DocumentLocator,
    ImageLimits, ImageMime, IndexAggregate, IndexAggregateStatus, IndexCorrectionConflictState,
    IndexCorrectionReviewDto, IndexCorrectionValueKind, IndexFailureCode, IndexPageBlockKind,
    IndexPageBlockReviewDto, IndexPageReviewDto, IndexPageStatus, IndexPageStatusCountsDto,
    IndexQualityReason, IndexReviewReason, IndexRunAggregateDto, IndexRunStatus, IndexTableCellDto,
    LearningEvent, LearningRequest, LocalTextQuality, NormalizedBookInput, NormalizedRect,
    OnboardingStateDto, OnboardingStep, PageAnalysisBlockKind, ProviderCapability,
    ProviderCapabilityRegistryDto, ProviderModelCapability, ProviderOperationConsent,
    ProviderOperationConsentCategory, ProviderOperationConsentDecision, ProviderPageAnalysis,
    ProviderProfileSummary, RemoteCleanupStatus, SafeIndexErrorDto, TeachingInstructionDto,
    UiLanguage, UnifiedChatRequest, UnifiedMessage, UnifiedRole, UnifiedStreamEvent,
    UntrustedNormalizedRect, UntrustedPageAnalysis, UntrustedPageBlock, UntrustedTableCell,
    UpdateTeachingInstruction, ValidationResult, VisionAssetMeta, stable_block_id,
    stable_index_page_block_id, stable_index_page_id, stable_index_search_chunk_id,
    stable_section_id,
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
fn indexing_ids_are_application_generated_and_namespace_stable() {
    let book = uuid::uuid!("4f9a2c86-0da8-4dd4-a255-39b4cff89c66");
    let run = uuid::uuid!("e74fb7e3-4f9c-4a3f-ae87-a8204dff4d61");
    let page = stable_index_page_id(run, 7);
    let block = stable_index_page_block_id(book, 7, run, "page-analysis-v1", 3);
    let chunk = stable_index_search_chunk_id(block, ContentSource::AiTranscribed, 0);

    assert_eq!(page, stable_index_page_id(run, 7));
    assert_ne!(page, stable_index_page_id(run, 8));
    assert_eq!(block.get_version_num(), 5);
    assert_eq!(chunk.get_version_num(), 5);
    assert_ne!(
        chunk,
        stable_index_search_chunk_id(block, ContentSource::AiDescription, 0)
    );
}

#[test]
fn indexing_statuses_round_trip_exact_and_safe_dtos_hide_attempt_ownership() {
    let statuses = [
        IndexPageStatus::NotRequired,
        IndexPageStatus::Queued,
        IndexPageStatus::Rendering,
        IndexPageStatus::Sending,
        IndexPageStatus::Parsing,
        IndexPageStatus::Validating,
        IndexPageStatus::Indexed,
        IndexPageStatus::NeedsReview,
        IndexPageStatus::Failed,
        IndexPageStatus::Cancelled,
    ];
    let serialized = statuses
        .into_iter()
        .map(|status| serde_json::to_value(status).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        serialized,
        [
            serde_json::json!("not_required"),
            serde_json::json!("queued"),
            serde_json::json!("rendering"),
            serde_json::json!("sending"),
            serde_json::json!("parsing"),
            serde_json::json!("validating"),
            serde_json::json!("indexed"),
            serde_json::json!("needs_review"),
            serde_json::json!("failed"),
            serde_json::json!("cancelled"),
        ]
    );

    let timestamp = chrono::DateTime::parse_from_rfc3339("2026-08-04T00:00:00.000Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    let review = IndexPageReviewDto {
        id: uuid::Uuid::new_v4(),
        run_id: uuid::Uuid::new_v4(),
        book_id: uuid::Uuid::new_v4(),
        page_number: 1,
        quality_reason: IndexQualityReason::NoText,
        status: IndexPageStatus::Failed,
        review_reason: None,
        safe_error: Some(SafeIndexErrorDto {
            code: IndexFailureCode::IndexAttemptInterrupted,
            message: IndexFailureCode::IndexAttemptInterrupted
                .safe_message()
                .to_owned(),
            retryable: true,
        }),
        content_version: 0,
        blocks: Vec::new(),
        corrections: Vec::new(),
        updated_at: timestamp,
    };
    let json = serde_json::to_string(&review).unwrap();
    for forbidden in [
        "attemptId",
        "sourceSha256",
        "renderSha256",
        "responseSha256",
        "encryptedReference",
        "remoteResource",
        "imageBytes",
        "providerBody",
    ] {
        assert!(!json.contains(forbidden), "safe DTO leaked {forbidden}");
    }
}

#[test]
fn library_index_aggregate_is_safe_and_uses_only_frozen_statuses() {
    let statuses = [
        BookIndexAggregateStatus::NotRequired,
        BookIndexAggregateStatus::Ready,
        BookIndexAggregateStatus::Partial,
        BookIndexAggregateStatus::NeedsReview,
        BookIndexAggregateStatus::Failed,
    ];
    assert_eq!(
        statuses
            .into_iter()
            .map(|status| serde_json::to_value(status).unwrap())
            .collect::<Vec<_>>(),
        [
            serde_json::json!("not_required"),
            serde_json::json!("ready"),
            serde_json::json!("partial"),
            serde_json::json!("needs_review"),
            serde_json::json!("failed"),
        ]
    );

    let aggregate = IndexAggregate {
        status: BookIndexAggregateStatus::Ready,
        total_pages: 3,
        indexed_pages: 2,
        review_pages: 0,
        failed_pages: 0,
    };
    let serialized = serde_json::to_string(&aggregate).unwrap();
    for forbidden in ["source", "path", "content", "attempt", "provider", "sha256"] {
        assert!(!serialized.to_lowercase().contains(forbidden));
    }
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
    IndexAggregate::export_all(&config).unwrap();
    BookIndexAggregateStatus::export_all(&config).unwrap();
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
    IndexRunStatus::export_all(&config).unwrap();
    IndexPageStatus::export_all(&config).unwrap();
    IndexQualityReason::export_all(&config).unwrap();
    IndexAggregateStatus::export_all(&config).unwrap();
    IndexFailureCode::export_all(&config).unwrap();
    IndexReviewReason::export_all(&config).unwrap();
    SafeIndexErrorDto::export_all(&config).unwrap();
    IndexPageStatusCountsDto::export_all(&config).unwrap();
    IndexRunAggregateDto::export_all(&config).unwrap();
    IndexPageReviewDto::export_all(&config).unwrap();
    ContentSource::export_all(&config).unwrap();
    IndexPageBlockKind::export_all(&config).unwrap();
    IndexCorrectionValueKind::export_all(&config).unwrap();
    IndexCorrectionConflictState::export_all(&config).unwrap();
    IndexTableCellDto::export_all(&config).unwrap();
    IndexPageBlockReviewDto::export_all(&config).unwrap();
    IndexCorrectionReviewDto::export_all(&config).unwrap();
    AppSettingsDto::export_all(&config).unwrap();
    OnboardingStep::export_all(&config).unwrap();
    LocalTextQuality::export_all(&config).unwrap();
    OnboardingStateDto::export_all(&config).unwrap();
    UiLanguage::export_all(&config).unwrap();
    TeachingInstructionDto::export_all(&config).unwrap();
    UpdateTeachingInstruction::export_all(&config).unwrap();
}
