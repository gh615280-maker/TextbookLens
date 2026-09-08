use std::collections::BTreeMap;

use textbooklens_lib::db::{
    conversations::BookConversationHistoryDto, messages::ConversationMessageDto,
};
use textbooklens_lib::domain::{
    ActiveOperationKind, ActiveOperationSummaryDto, AiOperation, AnnotationDto, AppSettingsDto,
    BackupSummaryDto, BlockKind, BookFormat, BookIndexAggregateStatus,
    BookLearningPreparationRiskFlag, BookLearningPreparationSummary, BookSummary,
    CapabilitySupport, Citation, CitationReviewStatus, ClearAllDataStatusCode,
    ClearAllDataSummaryDto, ContentAnchor, ContentSource, ConversationAnchorKind, ConversationDto,
    CredentialStatus, DocumentLocator, ImageLimits, ImageMime, IndexAggregate,
    IndexAggregateStatus, IndexCorrectionConflictState, IndexCorrectionReviewDto,
    IndexCorrectionValueKind, IndexFailureCode, IndexPageBlockKind, IndexPageBlockReviewDto,
    IndexPageReviewDto, IndexPageStatus, IndexPageStatusCountsDto, IndexQualityReason,
    IndexReviewReason, IndexRunAggregateDto, IndexRunStatus, IndexTableCellDto, LearningEvent,
    LearningOverview, LearningOverviewActivity, LearningOverviewErrorCode,
    LearningOverviewErrorDto, LearningOverviewSection, LearningOverviewSource,
    LearningOverviewSourceSummary, LearningRequest, LearningRequestEvent,
    LearningRequestEventPayload, LearningRequestSnapshot, LearningRequestStatus, LearningUsage,
    LocalTextQuality, MaintenanceErrorCode, MaintenanceErrorDto, MaintenanceStatusCode,
    MaintenanceStatusDto, NormalizedBookInput, NormalizedRect, OnboardingStateDto, OnboardingStep,
    PageAnalysisBlockKind, PanelGeometry, PrepareBookLearningRequestMetadata, ProviderCapability,
    ProviderCapabilityRegistryDto, ProviderModelCapability, ProviderOperationConsent,
    ProviderOperationConsentCategory, ProviderOperationConsentDecision, ProviderPageAnalysis,
    ProviderProfileSummary, RegionAnchor, RegionLocator, RemoteCleanupStatus,
    RestoreBackupStatusCode, RestoreBackupSummaryDto, SafeIndexErrorDto, SafeLearningError,
    StorageCategory, StorageCategoryUsageDto, StorageUsageDto, TeachingInstructionDto, UiLanguage,
    UnifiedChatRequest, UnifiedMessage, UnifiedRole, UnifiedStreamEvent, UntrustedNormalizedRect,
    UntrustedPageAnalysis, UntrustedPageBlock, UntrustedTableCell, UpdateTeachingInstruction,
    ValidationResult, VisionAssetMeta, stable_block_id, stable_index_page_block_id,
    stable_index_page_id, stable_index_search_chunk_id, stable_section_id,
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

    let rect = NormalizedRect::new(0.0, 0.0, 0.5, 0.5).unwrap();
    assert!(RegionLocator::pdf(0).is_err());
    assert!(
        RegionAnchor::new(
            RegionLocator::pdf(1).unwrap(),
            rect.clone(),
            "A".repeat(64),
            None,
        )
        .is_err()
    );
    assert!(
        serde_json::from_value::<ContentAnchor>(serde_json::json!({
            "kind": "region",
            "region": {
                "locator": { "format": "pdf", "page": 1 },
                "rect": { "x": 0.0, "y": 0.0, "width": 0.0, "height": 0.5 },
                "contentSha256": "a".repeat(64),
                "textFallback": null
            }
        }))
        .is_err()
    );
}

#[test]
fn learning_panel_dtos_are_exact_bounded_and_redacted() {
    let request_id = uuid::uuid!("f9369a97-d46c-4cf2-8bb0-7c595ea72de4");
    let conversation_id = uuid::uuid!("7ce80142-c696-4713-906b-2f1277fce3f7");
    let usage = LearningUsage::new(Some(12), Some(7)).unwrap();
    let snapshot = LearningRequestSnapshot::new(
        request_id,
        Some(conversation_id),
        LearningRequestStatus::Completed,
        "visible-snapshot-sentinel".to_owned(),
        Some(usage.clone()),
        None,
        4,
    )
    .unwrap();
    assert_eq!(
        serde_json::to_value(&snapshot).unwrap(),
        serde_json::json!({
            "requestId": request_id,
            "conversationId": conversation_id,
            "status": "completed",
            "text": "visible-snapshot-sentinel",
            "usage": { "inputTokens": 12, "outputTokens": 7 },
            "safeError": null,
            "lastSeq": 4
        })
    );
    assert!(!format!("{snapshot:?}").contains("visible-snapshot-sentinel"));

    let events = [
        LearningRequestEvent::new(request_id, 1, LearningRequestEventPayload::Preparing).unwrap(),
        LearningRequestEvent::new(
            request_id,
            2,
            LearningRequestEventPayload::text_delta("visible-event-sentinel".to_owned()).unwrap(),
        )
        .unwrap(),
        LearningRequestEvent::new(
            request_id,
            3,
            LearningRequestEventPayload::usage(Some(12), Some(7)).unwrap(),
        )
        .unwrap(),
        LearningRequestEvent::new(
            request_id,
            4,
            LearningRequestEventPayload::Completed { conversation_id },
        )
        .unwrap(),
    ];
    assert_eq!(
        events
            .iter()
            .map(|event| serde_json::to_value(event).unwrap())
            .collect::<Vec<_>>(),
        vec![
            serde_json::json!({
                "requestId": request_id,
                "seq": 1,
                "event": { "type": "preparing" }
            }),
            serde_json::json!({
                "requestId": request_id,
                "seq": 2,
                "event": { "type": "text_delta", "text": "visible-event-sentinel" }
            }),
            serde_json::json!({
                "requestId": request_id,
                "seq": 3,
                "event": { "type": "usage", "inputTokens": 12, "outputTokens": 7 }
            }),
            serde_json::json!({
                "requestId": request_id,
                "seq": 4,
                "event": { "type": "completed", "conversationId": conversation_id }
            }),
        ]
    );
    assert!(!format!("{:?}", events[1]).contains("visible-event-sentinel"));

    assert_eq!(
        [
            LearningRequestStatus::Preparing,
            LearningRequestStatus::Streaming,
            LearningRequestStatus::Completed,
            LearningRequestStatus::Failed,
            LearningRequestStatus::Cancelled,
        ]
        .into_iter()
        .map(|status| serde_json::to_value(status).unwrap())
        .collect::<Vec<_>>(),
        [
            serde_json::json!("preparing"),
            serde_json::json!("streaming"),
            serde_json::json!("completed"),
            serde_json::json!("failed"),
            serde_json::json!("cancelled"),
        ]
    );

    assert!(LearningUsage::new(None, None).is_err());
    assert!(LearningUsage::new(Some(u64::from(u32::MAX) + 1), None).is_err());
    assert!(SafeLearningError::new("provider body").is_err());
    assert!(
        LearningRequestEvent::new(request_id, 0, LearningRequestEventPayload::Cancelled).is_err()
    );
    assert!(LearningRequestEventPayload::text_delta("x".repeat(4 * 1024 * 1024 + 1)).is_err());
    assert!(
        LearningRequestSnapshot::new(
            request_id,
            None,
            LearningRequestStatus::Completed,
            "answer".to_owned(),
            None,
            None,
            1,
        )
        .is_err()
    );
}

#[test]
fn learning_overview_contract_is_source_explicit_content_free_and_debug_redacted() {
    let book_id = uuid::uuid!("4f9a2c86-0da8-4dd4-a255-39b4cff89c66");
    let section_id = stable_section_id(book_id, 0);
    let visible_title = "VISIBLE_SECTION_TITLE_SENTINEL";
    let sources = LearningOverviewSource::ALL
        .into_iter()
        .map(LearningOverviewSourceSummary::empty)
        .collect::<Vec<_>>();
    let overview = LearningOverview {
        book_id,
        format: BookFormat::Pdf,
        teaching_instruction_configured: true,
        section_count: 1,
        sections: vec![LearningOverviewSection {
            id: section_id,
            parent_id: None,
            ordinal: 0,
            title: visible_title.to_owned(),
            local_text_item_count: 0,
            user_note_count: 0,
            completed_conversation_count: 0,
            completed_exchange_count: 0,
        }],
        sources,
        activity: LearningOverviewActivity {
            user_note_count: 0,
            completed_conversation_count: 0,
            completed_exchange_count: 0,
            citation_count: 0,
        },
    };
    let value = serde_json::to_value(&overview).unwrap();
    assert_eq!(value["bookId"], serde_json::json!(book_id));
    assert_eq!(value["sections"][0]["title"], visible_title);
    assert_eq!(
        value["sources"]
            .as_array()
            .unwrap()
            .iter()
            .map(|source| source["source"].as_str().unwrap())
            .collect::<Vec<_>>(),
        [
            "local_text",
            "ai_transcribed",
            "ai_description",
            "user_corrected",
            "user_note",
            "history_summary",
        ]
    );
    let serialized = value.to_string();
    for forbidden in [
        "PRIVATE_TEXTBOOK_BODY_SENTINEL",
        "PRIVATE_NOTE_BODY_SENTINEL",
        "PRIVATE_PROMPT_SENTINEL",
        "PRIVATE_ANSWER_SENTINEL",
        "PRIVATE_ABSOLUTE_PATH_SENTINEL",
        "credential",
        "providerProfile",
        "attemptId",
        "requestId",
        "runId",
    ] {
        assert!(!serialized.contains(forbidden));
    }
    assert!(!format!("{overview:?}").contains(visible_title));

    let error = LearningOverviewErrorDto::new(LearningOverviewErrorCode::DataInvalid);
    assert_eq!(
        serde_json::to_value(error).unwrap(),
        serde_json::json!({ "code": "LEARNING_OVERVIEW_DATA_INVALID" })
    );
}

#[test]
fn panel_geometry_rejects_non_finite_and_out_of_range_values() {
    assert_eq!(
        serde_json::to_value(PanelGeometry::default()).unwrap(),
        serde_json::json!({
            "xRatio": 0.5,
            "yRatio": 0.5,
            "widthPx": 400.0,
            "heightPx": 520.0
        })
    );
    for geometry in [
        PanelGeometry::new(f64::NAN, 0.5, 400.0, 520.0),
        PanelGeometry::new(0.5, f64::INFINITY, 400.0, 520.0),
        PanelGeometry::new(-0.1, 0.5, 400.0, 520.0),
        PanelGeometry::new(0.5, 1.1, 400.0, 520.0),
        PanelGeometry::new(0.5, 0.5, 319.9, 520.0),
        PanelGeometry::new(0.5, 0.5, 400.0, 8192.1),
    ] {
        assert!(geometry.is_err());
    }
}

#[test]
fn maintenance_bindings_are_bounded_structural_and_path_free() {
    let status = MaintenanceStatusDto {
        code: MaintenanceStatusCode::MaintenanceWaiting,
        active_operations: vec![ActiveOperationSummaryDto {
            kind: ActiveOperationKind::Indexing,
            count: 2,
        }],
    };
    assert_eq!(
        serde_json::to_value(&status).unwrap(),
        serde_json::json!({
            "code": "MAINTENANCE_WAITING",
            "activeOperations": [{ "kind": "indexing", "count": 2 }]
        })
    );
    let error = MaintenanceErrorDto {
        code: MaintenanceErrorCode::StorageEntryUnsafe,
        active_operations: status.active_operations,
    };
    let serialized = serde_json::to_string(&error).unwrap();
    for forbidden in [
        "path",
        "title",
        "content",
        "requestOutput",
        "providerPayload",
        "profileId",
        "internalUuid",
        "image",
        "credential",
    ] {
        assert!(
            !serialized
                .to_ascii_lowercase()
                .contains(&forbidden.to_ascii_lowercase())
        );
    }

    let usage = StorageUsageDto {
        total_bytes: 3,
        total_file_count: 1,
        categories: vec![StorageCategoryUsageDto {
            category: StorageCategory::Database,
            bytes: 3,
            file_count: 1,
        }],
    };
    assert_eq!(
        serde_json::to_value(usage).unwrap(),
        serde_json::json!({
            "totalBytes": 3,
            "totalFileCount": 1,
            "categories": [{ "category": "database", "bytes": 3, "fileCount": 1 }]
        })
    );

    let backup = BackupSummaryDto {
        format_version: 1,
        archive_bytes: 4_096,
        entry_count: 2,
    };
    let serialized = serde_json::to_string(&backup).unwrap();
    assert_eq!(
        serde_json::to_value(backup).unwrap(),
        serde_json::json!({
            "formatVersion": 1,
            "archiveBytes": 4_096,
            "entryCount": 2
        })
    );
    for forbidden in ["path", "title", "content", "profile", "credential", "id"] {
        assert!(!serialized.to_ascii_lowercase().contains(forbidden));
    }

    let restore = RestoreBackupSummaryDto {
        status: RestoreBackupStatusCode::ReadyToRestart,
        restart_required: true,
        ai_configuration_required: true,
    };
    let restore_serialized = serde_json::to_string(&restore).unwrap();
    assert_eq!(
        serde_json::to_value(&restore).unwrap(),
        serde_json::json!({
            "status": "RESTORE_READY_TO_RESTART",
            "restartRequired": true,
            "aiConfigurationRequired": true
        })
    );
    let clear = ClearAllDataSummaryDto {
        status: ClearAllDataStatusCode::CredentialCleanupRequired,
        restart_required: false,
    };
    let serialized = serde_json::to_string(&clear).unwrap();
    assert_eq!(
        serde_json::to_value(clear).unwrap(),
        serde_json::json!({
            "status": "CLEAR_CREDENTIAL_CLEANUP_REQUIRED",
            "restartRequired": false
        })
    );
    for forbidden in [
        "path",
        "title",
        "content",
        "profileid",
        "credentialtarget",
        "credentialkey",
        "providertarget",
        "providerid",
        "archive",
    ] {
        assert!(!serialized.to_ascii_lowercase().contains(forbidden));
        assert!(!restore_serialized.to_ascii_lowercase().contains(forbidden));
    }
}

#[test]
fn book_learning_bindings_accept_only_question_routing_and_return_content_free_summaries() {
    let book_id = uuid::Uuid::new_v4();
    let conversation_id = uuid::Uuid::new_v4();
    let parsed: PrepareBookLearningRequestMetadata = serde_json::from_value(serde_json::json!({
        "kind": "continue",
        "bookId": book_id,
        "conversationId": conversation_id,
        "question": "Synthetic bounded question?"
    }))
    .unwrap();
    assert_eq!(parsed.book_id(), book_id);
    assert_eq!(parsed.conversation_id(), Some(conversation_id));
    let debug = format!("{parsed:?}");
    assert!(!debug.contains("Synthetic bounded question?"));
    assert!(!debug.contains(&book_id.to_string()));
    assert!(!debug.contains(&conversation_id.to_string()));
    for forbidden in [
        "toc",
        "retrievalSnippets",
        "history",
        "teachingInstruction",
        "providerProfileId",
        "modelId",
        "prompt",
        "citations",
        "bookContent",
    ] {
        let mut value = serde_json::json!({
            "kind": "new",
            "bookId": book_id,
            "question": "Synthetic bounded question?"
        });
        value
            .as_object_mut()
            .unwrap()
            .insert(forbidden.to_owned(), serde_json::json!("PRIVATE_SENTINEL"));
        assert!(serde_json::from_value::<PrepareBookLearningRequestMetadata>(value).is_err());
    }

    let summary = BookLearningPreparationSummary {
        preparation_id: uuid::Uuid::new_v4(),
        provider_display_name: "Provider".to_owned(),
        profile_display_name: "Profile".to_owned(),
        model_display_name: "Model".to_owned(),
        estimated_input_tokens: 900,
        source_count: 5,
        citation_count: 2,
        omitted_source_count: 1,
        risk_flags: vec![BookLearningPreparationRiskFlag::CostRisk],
        requires_blocking_confirmation: true,
        expires_at: chrono::DateTime::parse_from_rfc3339("2026-08-06T00:05:00.000Z")
            .unwrap()
            .with_timezone(&chrono::Utc),
    };
    let serialized = serde_json::to_string(&summary).unwrap();
    for forbidden in [
        "PRIVATE_SENTINEL",
        "question",
        "title",
        "snippet",
        "history",
        "prompt",
        "locator",
        "bookId",
        "conversationId",
        "providerProfileId",
        "modelId",
        "path",
    ] {
        assert!(!serialized.contains(forbidden));
    }
}

#[test]
fn export_bindings() {
    let output_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../src/lib/generated");
    std::fs::create_dir_all(&output_dir).unwrap();
    let config = Config::new().with_out_dir(output_dir.clone());

    BookSummary::export_all(&config).unwrap();
    PrepareBookLearningRequestMetadata::export_all(&config).unwrap();
    BookLearningPreparationRiskFlag::export_all(&config).unwrap();
    BookLearningPreparationSummary::export_all(&config).unwrap();
    LearningOverviewSource::export_all(&config).unwrap();
    LearningOverviewSourceSummary::export_all(&config).unwrap();
    LearningOverviewSection::export_all(&config).unwrap();
    LearningOverviewActivity::export_all(&config).unwrap();
    LearningOverview::export_all(&config).unwrap();
    LearningOverviewErrorCode::export_all(&config).unwrap();
    LearningOverviewErrorDto::export_all(&config).unwrap();
    IndexAggregate::export_all(&config).unwrap();
    BookIndexAggregateStatus::export_all(&config).unwrap();
    NormalizedBookInput::export_all(&config).unwrap();
    ContentAnchor::export_all(&config).unwrap();
    AnnotationDto::export_all(&config).unwrap();
    ConversationDto::export_all(&config).unwrap();
    ConversationMessageDto::export_all(&config).unwrap();
    BookConversationHistoryDto::export_all(&config).unwrap();
    ConversationAnchorKind::export_all(&config).unwrap();
    CitationReviewStatus::export_all(&config).unwrap();
    Citation::export_all(&config).unwrap();
    LearningRequest::export_all(&config).unwrap();
    LearningEvent::export_all(&config).unwrap();
    LearningRequestStatus::export_all(&config).unwrap();
    LearningUsage::export_all(&config).unwrap();
    SafeLearningError::export_all(&config).unwrap();
    LearningRequestSnapshot::export_all(&config).unwrap();
    LearningRequestEventPayload::export_all(&config).unwrap();
    LearningRequestEvent::export_all(&config).unwrap();
    PanelGeometry::export_all(&config).unwrap();
    CapabilitySupport::export_all(&config).unwrap();
    AiOperation::export_all(&config).unwrap();
    ImageLimits::export_all(&config).unwrap();
    ProviderModelCapability::export_all(&config).unwrap();
    ProviderCapability::export_all(&config).unwrap();
    ProviderCapabilityRegistryDto::export_all(&config).unwrap();
    CredentialStatus::export_all(&config).unwrap();
    textbooklens_lib::domain::LocalModelConnectResult::export_all(&config).unwrap();
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
    ActiveOperationKind::export_all(&config).unwrap();
    ActiveOperationSummaryDto::export_all(&config).unwrap();
    MaintenanceStatusCode::export_all(&config).unwrap();
    MaintenanceStatusDto::export_all(&config).unwrap();
    MaintenanceErrorCode::export_all(&config).unwrap();
    MaintenanceErrorDto::export_all(&config).unwrap();
    StorageCategory::export_all(&config).unwrap();
    StorageCategoryUsageDto::export_all(&config).unwrap();
    StorageUsageDto::export_all(&config).unwrap();
    BackupSummaryDto::export_all(&config).unwrap();
    RestoreBackupStatusCode::export_all(&config).unwrap();
    RestoreBackupSummaryDto::export_all(&config).unwrap();
    ClearAllDataStatusCode::export_all(&config).unwrap();
    ClearAllDataSummaryDto::export_all(&config).unwrap();

    let overview_binding =
        std::fs::read_to_string(output_dir.join("overview.ts")).expect("overview binding");
    assert!(overview_binding.contains("history_summary"));
    assert!(overview_binding.contains("quoteableAsTextbook"));
    for forbidden in [
        "PRIVATE_TS_BINDING_SENTINEL",
        "noteBody",
        "prompt",
        "answer",
        "storedPath",
        "credential",
        "providerProfileId",
        "attemptId",
        "requestId",
        "runId",
    ] {
        assert!(!overview_binding.contains(forbidden));
    }
}
