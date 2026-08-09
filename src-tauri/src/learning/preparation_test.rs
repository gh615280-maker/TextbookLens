use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use async_trait::async_trait;
use chrono::{TimeZone, Utc};
use parking_lot::Mutex;
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::{
    ai::registry::ProviderCapabilityRegistry,
    credentials::MemoryCredentialStore,
    db::Database,
    domain::{
        ContentAnchor, DocumentLocator, NormalizedRect, ProviderOperationConsent,
        ProviderOperationConsentCategory, ProviderOperationConsentDecision, RegionAnchor,
        RegionLocator, SelectionAnchor, TextQuote,
    },
    errors::{AppError, AppErrorCode, AppResult},
};

use super::{
    captures::{OwnedCaptureBytes, REGION_CAPTURE_SCHEMA_VERSION, RegionCaptureMetadata},
    preparation::{
        InvalidateLearningPreparations, LearningAction, LearningAuthorizationDecision,
        LearningContentKind, LearningInvalidationReason, LearningPreparationService,
        PREPARATION_TTL, PreparationRegistry, PreparationRiskFlag, PreparationSensitiveAccess,
        PrepareLearningRequestMetadata,
    },
};

const MODEL_ID: &str = "gpt-5.6";
const SELECTED_TEXT: &str = "PREPARATION_TEXT_SENTINEL spectral beacon selection";

#[test]
fn preparation_requires_mandatory_packing_before_sensitive_access() {
    let fixture = Fixture::new();
    tauri::async_runtime::block_on(async {
        sqlx::query("UPDATE provider_profiles SET context_window_tokens = 4096 WHERE id = ?")
            .bind(fixture.profile_id.to_string())
            .execute(fixture.pool())
            .await
            .unwrap();
        let error = fixture
            .service
            .prepare(fixture.text_metadata())
            .await
            .expect_err("mandatory prompt cannot fit");
        assert_eq!(error.code, AppErrorCode::ContextTooLarge);
        assert_eq!(fixture.access.credential_calls(), 0);
        assert_eq!(fixture.access.consent_calls(), 0);
        assert_eq!(fixture.registry.stats().count, 0);
        assert_eq!(fixture.registry.stats().total_image_bytes, 0);
    });
}

#[test]
fn preparation_fails_safely_for_missing_credential_consent_profile_and_capability() {
    let fixture = Fixture::new();
    tauri::async_runtime::block_on(async {
        fixture.access.set_credential_available(false);
        let error = fixture
            .service
            .prepare(fixture.text_metadata())
            .await
            .expect_err("missing credential");
        assert_eq!(error.code, AppErrorCode::CredentialStoreError);
        assert_eq!(fixture.access.credential_calls(), 1);
        assert_eq!(fixture.access.consent_calls(), 0);

        fixture.access.reset_counts();
        fixture.access.set_credential_available(true);
        fixture
            .access
            .remove_consent(ProviderOperationConsentCategory::CostRisk);
        let error = fixture
            .service
            .prepare(fixture.text_metadata())
            .await
            .expect_err("missing consent row");
        assert_eq!(error.code, AppErrorCode::DatabaseError);
        assert_eq!(fixture.access.credential_calls(), 1);
        assert_eq!(fixture.access.consent_calls(), 1);
        fixture.access.reset_consents(fixture.profile_id);

        let mut wrong_model = fixture.text_metadata();
        wrong_model.model_id = "frontend-substituted-model".to_owned();
        let error = fixture
            .service
            .prepare(wrong_model)
            .await
            .expect_err("frontend cannot replace profile model");
        assert_eq!(error.code, AppErrorCode::InvalidInput);

        sqlx::query(
            "UPDATE provider_profiles SET provider_kind = 'deepseek', model_id = 'deepseek-v4-flash' WHERE id = ?",
        )
        .bind(fixture.profile_id.to_string())
        .execute(fixture.pool())
        .await
        .unwrap();
        let mut unsupported = fixture.visual_metadata(b"vision");
        unsupported.model_id = "deepseek-v4-flash".to_owned();
        let error = fixture
            .service
            .prepare(unsupported)
            .await
            .expect_err("vision must be explicitly supported");
        assert_eq!(error.stable_code(), "UNSUPPORTED_PROVIDER_CAPABILITY");

        sqlx::query(
            "UPDATE provider_profiles SET provider_kind = 'openai', model_id = ?, validated_at = NULL WHERE id = ?",
        )
        .bind(MODEL_ID)
        .bind(fixture.profile_id.to_string())
        .execute(fixture.pool())
        .await
        .unwrap();
        let error = fixture
            .service
            .prepare(fixture.text_metadata())
            .await
            .expect_err("unverified profile");
        assert_eq!(error.code, AppErrorCode::RequestConflict);
    });
}

#[test]
fn preparation_validates_cross_book_format_section_and_anchor_hash() {
    let fixture = Fixture::new();
    tauri::async_runtime::block_on(async {
        let (other_book, other_section) = seed_book(fixture.pool(), "c").await;
        let mut cross_book = fixture.text_metadata();
        cross_book.book_id = other_book;
        cross_book.section_id = other_section;
        let error = fixture
            .service
            .prepare(cross_book)
            .await
            .expect_err("anchor still owns the original section");
        assert_eq!(error.code, AppErrorCode::InvalidInput);

        let mut wrong_format = fixture.visual_metadata(b"wrong-format");
        if let ContentAnchor::Region { region } = &mut wrong_format.anchor {
            region.locator = RegionLocator::Epub {
                section_id: fixture.section_id,
                cfi: "epubcfi(/6/2)".to_owned(),
            };
        }
        let error = fixture
            .service
            .prepare(wrong_format)
            .await
            .expect_err("format mismatch");
        assert_eq!(error.code, AppErrorCode::InvalidInput);

        let mut wrong_section = fixture.visual_metadata(b"wrong-section");
        if let ContentAnchor::Region { region } = &mut wrong_section.anchor {
            region.locator = RegionLocator::Pdf { page: 2 };
        }
        let error = fixture
            .service
            .prepare(wrong_section)
            .await
            .expect_err("page is outside the selected section");
        assert_eq!(error.code, AppErrorCode::InvalidInput);

        let mut invalid_hash = fixture.visual_metadata(b"bad-hash");
        if let ContentAnchor::Region { region } = &mut invalid_hash.anchor {
            region.content_sha256 = "UPPERCASE_OR_SHORT".to_owned();
        }
        let error = fixture
            .service
            .prepare(invalid_hash)
            .await
            .expect_err("invalid anchor hash");
        assert_eq!(error.code, AppErrorCode::InvalidInput);

        let mut malformed_pdf = fixture.text_metadata();
        if let ContentAnchor::Text { selection } = &mut malformed_pdf.anchor {
            selection.locator = DocumentLocator::Pdf {
                start_page: 2,
                end_page: 1,
                rects_by_page: None,
            };
        }
        let error = fixture
            .service
            .prepare(malformed_pdf)
            .await
            .expect_err("malformed PDF locator");
        assert_eq!(error.code, AppErrorCode::InvalidInput);

        let mut oversized_quote_context = fixture.text_metadata();
        if let ContentAnchor::Text { selection } = &mut oversized_quote_context.anchor {
            selection.quote.prefix = "p".repeat(65);
        }
        let error = fixture
            .service
            .prepare(oversized_quote_context)
            .await
            .expect_err("quote context is bounded at the Rust boundary");
        assert_eq!(error.code, AppErrorCode::InvalidInput);
        assert_eq!(fixture.access.credential_calls(), 0);
    });
}

#[test]
fn preparation_validates_docx_block_ownership_and_offsets() {
    let fixture = Fixture::new();
    tauri::async_runtime::block_on(async {
        let block_id = block_id_for_text(fixture.pool(), fixture.book_id, SELECTED_TEXT).await;
        let text_length = u32::try_from(SELECTED_TEXT.chars().count()).unwrap();
        let section_locator = DocumentLocator::docx(block_id, 0, block_id, text_length).unwrap();
        sqlx::query("UPDATE books SET format = 'docx' WHERE id = ?")
            .bind(fixture.book_id.to_string())
            .execute(fixture.pool())
            .await
            .unwrap();
        sqlx::query("UPDATE sections SET locator_json = ? WHERE id = ? AND book_id = ?")
            .bind(serde_json::to_string(&section_locator).unwrap())
            .bind(fixture.section_id.to_string())
            .bind(fixture.book_id.to_string())
            .execute(fixture.pool())
            .await
            .unwrap();

        let mut valid = fixture.text_metadata();
        if let ContentAnchor::Text { selection } = &mut valid.anchor {
            selection.locator = section_locator;
        }
        let summary = fixture
            .service
            .prepare(valid)
            .await
            .expect("owned DOCX range");
        fixture.registry.discard(summary.preparation_id);

        let (other_book, _) = seed_book(fixture.pool(), "e").await;
        let foreign_block = block_id_for_text(fixture.pool(), other_book, SELECTED_TEXT).await;
        let mut cross_book_block = fixture.text_metadata();
        if let ContentAnchor::Text { selection } = &mut cross_book_block.anchor {
            selection.locator = DocumentLocator::Docx {
                start_block_id: block_id,
                start_offset: 0,
                end_block_id: foreign_block,
                end_offset: 1,
            };
        }
        let error = fixture
            .service
            .prepare(cross_book_block)
            .await
            .expect_err("foreign DOCX block");
        assert_eq!(error.code, AppErrorCode::InvalidInput);

        let mut out_of_bounds = fixture.text_metadata();
        if let ContentAnchor::Text { selection } = &mut out_of_bounds.anchor {
            selection.locator = DocumentLocator::Docx {
                start_block_id: block_id,
                start_offset: 0,
                end_block_id: block_id,
                end_offset: text_length + 1,
            };
        }
        let error = fixture
            .service
            .prepare(out_of_bounds)
            .await
            .expect_err("DOCX offset beyond block text");
        assert_eq!(error.code, AppErrorCode::InvalidInput);
    });
}

#[test]
fn preparation_summary_has_only_safe_exact_fields_and_normal_text_needs_no_confirmation() {
    let fixture = Fixture::new();
    tauri::async_runtime::block_on(async {
        let summary = fixture
            .service
            .prepare(fixture.text_metadata())
            .await
            .expect("text preparation");
        assert!(!summary.will_send_image);
        assert!(summary.risk_flags.is_empty());
        assert!(!summary.requires_blocking_confirmation);
        assert!(summary.source_count >= 1);
        assert!(summary.citation_count >= 1);
        assert!(summary.estimated_input_tokens > 0);
        assert_eq!(summary.provider_display_name, "OpenAI");
        assert_eq!(summary.profile_display_name, "Preparation profile");
        assert_eq!(summary.model_display_name, "GPT-5.6");

        let serialized = serde_json::to_value(&summary).unwrap();
        let keys = serialized
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            keys,
            std::collections::BTreeSet::from([
                "actionCategory",
                "citationCount",
                "estimatedInputTokens",
                "expiresAt",
                "modelDisplayName",
                "omittedSourceCount",
                "preparationId",
                "profileDisplayName",
                "providerDisplayName",
                "requiresBlockingConfirmation",
                "riskFlags",
                "sourceCount",
                "willSendImage",
            ])
        );
        let serialized = serialized.to_string();
        let sentinels = [
            SELECTED_TEXT.to_owned(),
            "original.pdf".to_owned(),
            "books/".to_owned(),
            "a".repeat(64),
            fixture.profile_id.to_string(),
            MODEL_ID.to_owned(),
        ];
        for sentinel in sentinels {
            assert!(!serialized.contains(&sentinel));
        }

        let request = fixture
            .service
            .consume(summary.preparation_id, None)
            .await
            .expect("one-time internal consume");
        assert_eq!(request.book_id(), fixture.book_id);
        assert_eq!(request.provider_profile_id(), fixture.profile_id);
        assert_eq!(request.model_id(), MODEL_ID);
        assert_eq!(
            request
                .prepared_prompt()
                .clone()
                .into_chat_request(MODEL_ID.to_owned(), 1)
                .expected_language
                .as_deref(),
            Some("zh-CN")
        );
        assert!(!request.packed_context().citations.is_empty());
        let debug = format!("{request:?}");
        assert!(!debug.contains(SELECTED_TEXT));
        assert!(!debug.contains(&fixture.profile_id.to_string()));
        assert!(!debug.contains(MODEL_ID));
    });
}

#[test]
fn preparation_cost_risk_is_profile_scoped_and_first_use_blocks() {
    let fixture = Fixture::new();
    tauri::async_runtime::block_on(async {
        sqlx::query("UPDATE app_settings SET context_mode = 'long' WHERE id = 1")
            .execute(fixture.pool())
            .await
            .unwrap();
        let large_selection = "L".repeat(33_500);
        let mut metadata = fixture.text_metadata();
        metadata.selected_text = Some(large_selection.clone());
        if let ContentAnchor::Text { selection } = &mut metadata.anchor {
            selection.quote.exact = large_selection;
        }
        let summary = fixture
            .service
            .prepare(metadata)
            .await
            .expect("long context");
        assert_eq!(summary.risk_flags, vec![PreparationRiskFlag::CostRisk]);
        assert!(summary.requires_blocking_confirmation);

        fixture.access.set_decision(
            ProviderOperationConsentCategory::CostRisk,
            ProviderOperationConsentDecision::SkipPrompt,
        );
        let large_selection = "M".repeat(33_500);
        let mut metadata = fixture.text_metadata();
        metadata.selected_text = Some(large_selection.clone());
        if let ContentAnchor::Text { selection } = &mut metadata.anchor {
            selection.quote.exact = large_selection;
        }
        let summary = fixture
            .service
            .prepare(metadata)
            .await
            .expect("skip prompt");
        assert!(!summary.requires_blocking_confirmation);
        let error = fixture
            .service
            .consume(summary.preparation_id, None)
            .await
            .expect_err("skip preference is not authorization");
        assert_eq!(error.code, AppErrorCode::RequestConflict);
    });
}

#[test]
fn preparation_visual_consent_ask_deny_allow_and_skip_preference_are_safe() {
    let fixture = Fixture::new();
    tauri::async_runtime::block_on(async {
        let bytes = include_bytes!("../../../fixtures/source/vision/tiny-blue.png").to_vec();
        let ask = fixture
            .service
            .prepare(fixture.visual_metadata(&bytes))
            .await
            .expect("visual ask");
        assert!(ask.will_send_image);
        assert_eq!(ask.risk_flags, vec![PreparationRiskFlag::ImageSend]);
        assert!(ask.requires_blocking_confirmation);
        assert_eq!(
            fixture
                .service
                .authorize(ask.preparation_id, LearningAuthorizationDecision::Deny)
                .await
                .unwrap(),
            None
        );
        assert_eq!(fixture.registry.stats().count, 0);

        fixture.access.set_decision(
            ProviderOperationConsentCategory::ImageSend,
            ProviderOperationConsentDecision::SkipPrompt,
        );
        let skipped = fixture
            .service
            .prepare(fixture.visual_metadata(&bytes))
            .await
            .expect("visual skip preference");
        assert!(!skipped.requires_blocking_confirmation);
        let arbitrary_token = Uuid::new_v4();
        let error = fixture
            .service
            .stage_region_capture(
                fixture.capture_metadata(skipped.preparation_id, arbitrary_token, &bytes),
                OwnedCaptureBytes::new(bytes.clone()),
            )
            .await
            .expect_err("skip preference cannot authorize upload");
        assert_eq!(error.code, AppErrorCode::RequestConflict);

        let token = fixture
            .service
            .authorize(skipped.preparation_id, LearningAuthorizationDecision::Allow)
            .await
            .unwrap()
            .expect("opaque authorization token");
        fixture
            .service
            .stage_region_capture(
                fixture.capture_metadata(skipped.preparation_id, token, &bytes),
                OwnedCaptureBytes::new(bytes.clone()),
            )
            .await
            .expect("fresh authorized staging");
        let request = fixture
            .service
            .consume(skipped.preparation_id, Some(token))
            .await
            .expect("authorized one-time consume");
        assert_eq!(request.capture().unwrap().bytes(), bytes);
        assert_eq!(fixture.registry.stats().count, 0);
        assert_eq!(fixture.registry.stats().total_image_bytes, 0);
    });
}

#[test]
fn preparation_rejects_reliable_text_images_and_consent_changes() {
    let fixture = Fixture::new();
    tauri::async_runtime::block_on(async {
        let reliable = fixture
            .service
            .prepare(fixture.reliable_region_metadata())
            .await
            .expect("reliable region");
        assert!(!reliable.will_send_image);
        assert!(!reliable.requires_blocking_confirmation);
        let error = fixture
            .service
            .authorize(
                reliable.preparation_id,
                LearningAuthorizationDecision::Allow,
            )
            .await
            .expect_err("reliable region has no image authorization");
        assert_eq!(error.code, AppErrorCode::InvalidInput);

        let bytes = png(2, 2, b"consent-change");
        let visual = fixture
            .service
            .prepare(fixture.visual_metadata(&bytes))
            .await
            .expect("visual preparation");
        let token = fixture
            .service
            .authorize(visual.preparation_id, LearningAuthorizationDecision::Allow)
            .await
            .unwrap()
            .unwrap();
        fixture.access.set_decision(
            ProviderOperationConsentCategory::ImageSend,
            ProviderOperationConsentDecision::SkipPrompt,
        );
        let error = fixture
            .service
            .stage_region_capture(
                fixture.capture_metadata(visual.preparation_id, token, &bytes),
                OwnedCaptureBytes::new(bytes),
            )
            .await
            .expect_err("changed consent invalidates authorization");
        assert_eq!(error.code, AppErrorCode::RequestConflict);
        assert_eq!(fixture.registry.stats().count, 1); // reliable text entry remains
    });
}

#[test]
fn preparation_ttl_boundary_discard_gc_and_invalidation_release_memory() {
    let fixture = Fixture::new();
    tauri::async_runtime::block_on(async {
        let summary = fixture
            .service
            .prepare(fixture.text_metadata())
            .await
            .unwrap();
        let boundary = Instant::now() + Duration::from_secs(1);
        fixture
            .registry
            .set_expiry_for_test(summary.preparation_id, boundary)
            .unwrap();
        assert_eq!(
            fixture
                .registry
                .garbage_collect_at(boundary - Duration::from_nanos(1)),
            0
        );
        assert_eq!(fixture.registry.stats().count, 1);
        assert_eq!(fixture.registry.garbage_collect_at(boundary), 1);
        assert_eq!(fixture.registry.stats().count, 0);
        assert!(fixture.registry.stats().total_text_bytes == 0);
        let error = fixture
            .service
            .consume(summary.preparation_id, None)
            .await
            .expect_err("expired preparation cannot revive");
        assert_eq!(error.code, AppErrorCode::RequestConflict);

        let first = fixture
            .service
            .prepare(fixture.text_metadata())
            .await
            .unwrap();
        fixture.registry.discard(first.preparation_id);
        fixture.registry.discard(first.preparation_id);
        assert_eq!(fixture.registry.stats().count, 0);

        let _ = fixture
            .service
            .prepare(fixture.text_metadata())
            .await
            .unwrap();
        let removed = fixture
            .registry
            .invalidate(InvalidateLearningPreparations {
                reason: LearningInvalidationReason::BookChange,
                book_id: Some(fixture.book_id),
                provider_profile_id: None,
            })
            .unwrap();
        assert_eq!(removed, 1);
        let _ = fixture
            .service
            .prepare(fixture.text_metadata())
            .await
            .unwrap();
        assert_eq!(
            fixture
                .registry
                .invalidate(InvalidateLearningPreparations {
                    reason: LearningInvalidationReason::ProfileChange,
                    book_id: None,
                    provider_profile_id: Some(fixture.profile_id),
                })
                .unwrap(),
            1
        );
        let _ = fixture
            .service
            .prepare(fixture.text_metadata())
            .await
            .unwrap();
        assert_eq!(
            fixture
                .registry
                .invalidate(InvalidateLearningPreparations {
                    reason: LearningInvalidationReason::RouteChange,
                    book_id: None,
                    provider_profile_id: None,
                })
                .unwrap(),
            1
        );
        assert_eq!(fixture.registry.stats().count, 0);
        assert_eq!(fixture.registry.stats().total_text_bytes, 0);
    });
}

#[test]
fn preparation_zeroizes_capture_on_discard_expire_invalidate_consume_and_error() {
    let fixture = Fixture::new();
    tauri::async_runtime::block_on(async {
        for disposition in [
            Disposition::Discard,
            Disposition::Expire,
            Disposition::Invalidate,
        ] {
            let bytes = png(2, 2, format!("ZEROIZE_{disposition:?}").as_bytes());
            let summary = fixture
                .service
                .prepare(fixture.visual_metadata(&bytes))
                .await
                .unwrap();
            let token = fixture
                .service
                .authorize(summary.preparation_id, LearningAuthorizationDecision::Allow)
                .await
                .unwrap()
                .unwrap();
            let probe = Arc::new(AtomicBool::new(false));
            fixture
                .service
                .stage_region_capture(
                    fixture.capture_metadata(summary.preparation_id, token, &bytes),
                    OwnedCaptureBytes::with_zeroize_probe(bytes, probe.clone()),
                )
                .await
                .unwrap();
            assert!(!probe.load(Ordering::SeqCst));
            match disposition {
                Disposition::Discard => fixture.registry.discard(summary.preparation_id),
                Disposition::Expire => {
                    let now = Instant::now();
                    fixture
                        .registry
                        .set_expiry_for_test(summary.preparation_id, now)
                        .unwrap();
                    assert_eq!(fixture.registry.garbage_collect_at(now), 1);
                }
                Disposition::Invalidate => {
                    fixture
                        .registry
                        .invalidate(InvalidateLearningPreparations {
                            reason: LearningInvalidationReason::DefaultChange,
                            book_id: None,
                            provider_profile_id: None,
                        })
                        .unwrap();
                }
            }
            assert!(probe.load(Ordering::SeqCst), "{disposition:?}");
        }

        let bytes = png(2, 2, b"ZEROIZE_CONSUME");
        let summary = fixture
            .service
            .prepare(fixture.visual_metadata(&bytes))
            .await
            .unwrap();
        let token = fixture
            .service
            .authorize(summary.preparation_id, LearningAuthorizationDecision::Allow)
            .await
            .unwrap()
            .unwrap();
        let probe = Arc::new(AtomicBool::new(false));
        fixture
            .service
            .stage_region_capture(
                fixture.capture_metadata(summary.preparation_id, token, &bytes),
                OwnedCaptureBytes::with_zeroize_probe(bytes, probe.clone()),
            )
            .await
            .unwrap();
        let request = fixture
            .service
            .consume(summary.preparation_id, Some(token))
            .await
            .unwrap();
        assert!(!probe.load(Ordering::SeqCst));
        drop(request);
        assert!(probe.load(Ordering::SeqCst));
    });
}

#[test]
fn preparation_concurrent_consume_has_one_winner_and_profile_switch_never_rebinds() {
    let fixture = Fixture::new();
    tauri::async_runtime::block_on(async {
        let summary = fixture
            .service
            .prepare(fixture.text_metadata())
            .await
            .unwrap();
        let left = fixture.service.clone();
        let right = fixture.service.clone();
        let (left, right) = tokio::join!(
            left.consume(summary.preparation_id, None),
            right.consume(summary.preparation_id, None)
        );
        assert_eq!(usize::from(left.is_ok()) + usize::from(right.is_ok()), 1);

        let stale = fixture
            .service
            .prepare(fixture.text_metadata())
            .await
            .unwrap();
        sqlx::query("UPDATE app_settings SET default_learning_profile_id = NULL WHERE id = 1")
            .execute(fixture.pool())
            .await
            .unwrap();
        let error = fixture
            .service
            .consume(stale.preparation_id, None)
            .await
            .expect_err("default/profile switch invalidates snapshot");
        assert_eq!(error.code, AppErrorCode::RequestConflict);
        assert_eq!(fixture.registry.stats().count, 0);
        let late = fixture
            .service
            .consume(stale.preparation_id, None)
            .await
            .expect_err("late consume cannot revive");
        assert_eq!(late.code, AppErrorCode::RequestConflict);
    });
}

#[test]
fn preparation_performs_no_provider_network_or_durable_learning_writes() {
    let fixture = Fixture::new();
    tauri::async_runtime::block_on(async {
        let before = protected_counts(fixture.pool()).await;
        let summary = fixture
            .service
            .prepare(fixture.text_metadata())
            .await
            .unwrap();
        fixture.registry.discard(summary.preparation_id);
        let after = protected_counts(fixture.pool()).await;
        assert_eq!(before, after);
        assert_eq!(fixture.access.provider_calls(), 0);
        assert!(fixture.access.credential_calls() > 0);
        assert!(fixture.access.consent_calls() > 0);
    });
}

#[derive(Clone, Copy, Debug)]
enum Disposition {
    Discard,
    Expire,
    Invalidate,
}

struct Fixture {
    _temporary: tempfile::TempDir,
    database: Database,
    book_id: Uuid,
    section_id: Uuid,
    profile_id: Uuid,
    registry: PreparationRegistry,
    access: Arc<CountingSensitiveAccess>,
    service: LearningPreparationService,
}

impl Fixture {
    fn new() -> Self {
        assert_eq!(PREPARATION_TTL, Duration::from_secs(5 * 60));
        let temporary = tempfile::tempdir().unwrap();
        let database = Database::open(temporary.path().join("library.sqlite3")).unwrap();
        let profile_id = Uuid::new_v4();
        let (book_id, section_id) = tauri::async_runtime::block_on(async {
            seed_profile(database.pool(), profile_id).await;
            seed_book(database.pool(), "a").await
        });
        let registry = PreparationRegistry::default();
        let access = Arc::new(CountingSensitiveAccess::new(profile_id));
        let service = LearningPreparationService::new(
            database.pool().clone(),
            registry.clone(),
            ProviderCapabilityRegistry::load_embedded().unwrap(),
            Arc::new(MemoryCredentialStore::new()),
        )
        .with_sensitive_access(access.clone());
        Self {
            _temporary: temporary,
            database,
            book_id,
            section_id,
            profile_id,
            registry,
            access,
            service,
        }
    }

    fn pool(&self) -> &SqlitePool {
        self.database.pool()
    }

    fn text_metadata(&self) -> PrepareLearningRequestMetadata {
        PrepareLearningRequestMetadata {
            book_id: self.book_id,
            section_id: self.section_id,
            provider_profile_id: self.profile_id,
            model_id: MODEL_ID.to_owned(),
            action: LearningAction::Explain,
            content_kind: LearningContentKind::TextSelection,
            anchor: ContentAnchor::Text {
                selection: SelectionAnchor {
                    locator: DocumentLocator::pdf(1, 1, None).unwrap(),
                    quote: TextQuote::new(
                        SELECTED_TEXT.to_owned(),
                        "prefix".to_owned(),
                        "suffix".to_owned(),
                    )
                    .unwrap(),
                    section_id: Some(self.section_id),
                },
            },
            selected_text: Some(SELECTED_TEXT.to_owned()),
            question: None,
            target_language: None,
        }
    }

    fn reliable_region_metadata(&self) -> PrepareLearningRequestMetadata {
        let text = "Reliable region textbook text";
        PrepareLearningRequestMetadata {
            book_id: self.book_id,
            section_id: self.section_id,
            provider_profile_id: self.profile_id,
            model_id: MODEL_ID.to_owned(),
            action: LearningAction::Example,
            content_kind: LearningContentKind::ReliableTextRegion,
            anchor: ContentAnchor::Region {
                region: RegionAnchor::new(
                    RegionLocator::pdf(1).unwrap(),
                    NormalizedRect::new(0.1, 0.1, 0.5, 0.2).unwrap(),
                    sha256_hex(text.as_bytes()),
                    Some(TextQuote::new(text.to_owned(), String::new(), String::new()).unwrap()),
                )
                .unwrap(),
            },
            selected_text: Some(text.to_owned()),
            question: None,
            target_language: None,
        }
    }

    fn visual_metadata(&self, bytes: &[u8]) -> PrepareLearningRequestMetadata {
        PrepareLearningRequestMetadata {
            book_id: self.book_id,
            section_id: self.section_id,
            provider_profile_id: self.profile_id,
            model_id: MODEL_ID.to_owned(),
            action: LearningAction::Derive,
            content_kind: LearningContentKind::VisualRegion,
            anchor: ContentAnchor::Region {
                region: RegionAnchor::new(
                    RegionLocator::pdf(1).unwrap(),
                    NormalizedRect::new(0.1, 0.1, 0.5, 0.5).unwrap(),
                    sha256_hex(bytes),
                    None,
                )
                .unwrap(),
            },
            selected_text: None,
            question: None,
            target_language: None,
        }
    }

    fn capture_metadata(
        &self,
        preparation_id: Uuid,
        operation_token: Uuid,
        bytes: &[u8],
    ) -> RegionCaptureMetadata {
        let hash = sha256_hex(bytes);
        RegionCaptureMetadata {
            preparation_id,
            operation_token,
            book_id: self.book_id,
            provider_profile_id: self.profile_id,
            model_id: MODEL_ID.to_owned(),
            anchor_content_sha256: hash.clone(),
            capture_sha256: hash,
            schema_version: REGION_CAPTURE_SCHEMA_VERSION,
            mime_type: "image/png".to_owned(),
            width: 2,
            height: 2,
            decoded_pixel_count: 4,
            encoded_byte_length: u64::try_from(bytes.len()).unwrap(),
        }
    }
}

struct CountingSensitiveAccess {
    credential_calls: AtomicUsize,
    consent_calls: AtomicUsize,
    provider_calls: AtomicUsize,
    credential_available: AtomicBool,
    consents: Mutex<Vec<ProviderOperationConsent>>,
    revision: AtomicUsize,
}

impl CountingSensitiveAccess {
    fn new(profile_id: Uuid) -> Self {
        Self {
            credential_calls: AtomicUsize::new(0),
            consent_calls: AtomicUsize::new(0),
            provider_calls: AtomicUsize::new(0),
            credential_available: AtomicBool::new(true),
            consents: Mutex::new(default_consents(profile_id, 0)),
            revision: AtomicUsize::new(1),
        }
    }

    fn credential_calls(&self) -> usize {
        self.credential_calls.load(Ordering::SeqCst)
    }

    fn consent_calls(&self) -> usize {
        self.consent_calls.load(Ordering::SeqCst)
    }

    fn provider_calls(&self) -> usize {
        self.provider_calls.load(Ordering::SeqCst)
    }

    fn reset_counts(&self) {
        self.credential_calls.store(0, Ordering::SeqCst);
        self.consent_calls.store(0, Ordering::SeqCst);
    }

    fn set_credential_available(&self, value: bool) {
        self.credential_available.store(value, Ordering::SeqCst);
    }

    fn reset_consents(&self, profile_id: Uuid) {
        let revision = self.revision.fetch_add(1, Ordering::SeqCst);
        *self.consents.lock() = default_consents(profile_id, revision);
    }

    fn remove_consent(&self, category: ProviderOperationConsentCategory) {
        self.consents
            .lock()
            .retain(|consent| consent.category != category);
    }

    fn set_decision(
        &self,
        category: ProviderOperationConsentCategory,
        decision: ProviderOperationConsentDecision,
    ) {
        let revision = self.revision.fetch_add(1, Ordering::SeqCst);
        let mut consents = self.consents.lock();
        let consent = consents
            .iter_mut()
            .find(|consent| consent.category == category)
            .unwrap();
        consent.decision = decision;
        consent.updated_at = timestamp(revision);
    }
}

#[async_trait]
impl PreparationSensitiveAccess for CountingSensitiveAccess {
    async fn require_credential(&self, _profile_id: Uuid) -> AppResult<()> {
        self.credential_calls.fetch_add(1, Ordering::SeqCst);
        if self.credential_available.load(Ordering::SeqCst) {
            Ok(())
        } else {
            Err(AppError::credential_store("synthetic missing credential"))
        }
    }

    async fn load_consents(
        &self,
        _pool: &SqlitePool,
        _profile_id: Uuid,
    ) -> AppResult<Vec<ProviderOperationConsent>> {
        self.consent_calls.fetch_add(1, Ordering::SeqCst);
        Ok(self.consents.lock().clone())
    }
}

fn default_consents(profile_id: Uuid, revision: usize) -> Vec<ProviderOperationConsent> {
    [
        ProviderOperationConsentCategory::ImageSend,
        ProviderOperationConsentCategory::AiIndex,
        ProviderOperationConsentCategory::CostRisk,
    ]
    .into_iter()
    .map(|category| ProviderOperationConsent {
        profile_id,
        category,
        decision: ProviderOperationConsentDecision::Ask,
        updated_at: timestamp(revision),
    })
    .collect()
}

fn timestamp(revision: usize) -> chrono::DateTime<Utc> {
    Utc.timestamp_opt(1_786_000_000 + i64::try_from(revision).unwrap(), 0)
        .single()
        .unwrap()
}

async fn seed_profile(pool: &SqlitePool, profile_id: Uuid) {
    let timestamp = "2026-08-05T00:00:00.000Z";
    sqlx::query(
        "INSERT INTO provider_profiles (id, provider_kind, display_name, model_id, context_window_tokens, is_active, created_at, updated_at, validated_at) VALUES (?, 'openai', 'Preparation profile', ?, 128000, 1, ?, ?, ?)",
    )
    .bind(profile_id.to_string())
    .bind(MODEL_ID)
    .bind(timestamp)
    .bind(timestamp)
    .bind(timestamp)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "UPDATE app_settings SET active_provider_profile_id = ?, default_learning_profile_id = ?, default_vision_profile_id = ? WHERE id = 1",
    )
    .bind(profile_id.to_string())
    .bind(profile_id.to_string())
    .bind(profile_id.to_string())
    .execute(pool)
    .await
    .unwrap();
}

async fn seed_book(pool: &SqlitePool, hash_character: &str) -> (Uuid, Uuid) {
    let book_id = Uuid::new_v4();
    let section_id = Uuid::new_v4();
    let timestamp = "2026-08-05T00:00:00.000Z";
    sqlx::query(
        "INSERT INTO books (id, sha256, title, format, original_filename, stored_path, import_status, created_at, updated_at) VALUES (?, ?, 'Preparation book', 'pdf', 'original.pdf', ?, 'ready', ?, ?)",
    )
    .bind(book_id.to_string())
    .bind(hash_character.repeat(64))
    .bind(format!("books/{book_id}/original.pdf"))
    .bind(timestamp)
    .bind(timestamp)
    .execute(pool)
    .await
    .unwrap();
    let locator = serde_json::to_string(&DocumentLocator::pdf(1, 1, None).unwrap()).unwrap();
    sqlx::query(
        "INSERT INTO sections (id, book_id, ordinal, title, locator_json) VALUES (?, ?, 0, 'Preparation section', ?)",
    )
    .bind(section_id.to_string())
    .bind(book_id.to_string())
    .bind(&locator)
    .execute(pool)
    .await
    .unwrap();
    for (ordinal, kind, text) in [
        (0_i64, "heading", "Preparation section"),
        (1, "paragraph", SELECTED_TEXT),
        (2, "paragraph", "Neighbor context for preparation"),
        (3, "paragraph", "Definition: immutable preparation"),
    ] {
        sqlx::query(
            "INSERT INTO blocks (id, book_id, section_id, ordinal, kind, plain_text, locator_json) VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(book_id.to_string())
        .bind(section_id.to_string())
        .bind(ordinal)
        .bind(kind)
        .bind(text)
        .bind(&locator)
        .execute(pool)
        .await
        .unwrap();
    }
    sqlx::query(
        "INSERT INTO search_chunks (id, book_id, section_id, ordinal, text, locator_json, token_estimate) VALUES (?, ?, ?, 0, 'Preparation spectral beacon context', ?, 10)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(book_id.to_string())
    .bind(section_id.to_string())
    .bind(locator)
    .execute(pool)
    .await
    .unwrap();
    (book_id, section_id)
}

async fn protected_counts(pool: &SqlitePool) -> Vec<i64> {
    let mut counts = Vec::new();
    for query in [
        "SELECT COUNT(*) FROM conversations",
        "SELECT COUNT(*) FROM messages",
        "SELECT COUNT(*) FROM annotations",
        "SELECT COUNT(*) FROM index_runs",
        "SELECT COUNT(*) FROM index_pages",
        "SELECT COUNT(*) FROM index_page_blocks",
        "SELECT COUNT(*) FROM index_search_chunks",
    ] {
        counts.push(
            sqlx::query_scalar::<_, i64>(query)
                .fetch_one(pool)
                .await
                .unwrap(),
        );
    }
    counts
}

async fn block_id_for_text(pool: &SqlitePool, book_id: Uuid, text: &str) -> Uuid {
    let id = sqlx::query_scalar::<_, String>(
        "SELECT id FROM blocks WHERE book_id = ? AND plain_text = ? ORDER BY ordinal LIMIT 1",
    )
    .bind(book_id.to_string())
    .bind(text)
    .fetch_one(pool)
    .await
    .unwrap();
    Uuid::parse_str(&id).unwrap()
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|value| format!("{value:02x}"))
        .collect()
}

fn png(width: u32, height: u32, suffix: &[u8]) -> Vec<u8> {
    let mut bytes = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR".to_vec();
    bytes.extend_from_slice(&width.to_be_bytes());
    bytes.extend_from_slice(&height.to_be_bytes());
    bytes.extend_from_slice(suffix);
    bytes
}
