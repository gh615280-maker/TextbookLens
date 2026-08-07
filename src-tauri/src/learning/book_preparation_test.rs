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
use secrecy::SecretString;
use sqlx::SqlitePool;
use tokio::sync::{Semaphore, oneshot};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{
    ai::{registry::ProviderCapabilityRegistry, runtime::ProviderRuntime},
    credentials::{CredentialStore, MemoryCredentialStore},
    db::{
        Database,
        corrections::{SaveIndexCorrection, correction_value_sha256, save_index_correction},
        indexing::{CreateIndexRun, create_index_run},
        providers,
    },
    domain::{
        ActiveOperationKind, BookFormat, ContentSource as DurableContentSource, DocumentLocator,
        IndexCorrectionValueKind, IndexPageBlockKind, IndexQualityReason, LearningRequestStatus,
        MaintenanceStatusCode, NormalizedRect, PrepareBookLearningRequestMetadata,
        ProviderOperationConsent, ProviderOperationConsentCategory,
        ProviderOperationConsentDecision, UiLanguage, UnifiedRole, stable_section_id,
    },
    errors::{AppError, AppErrorCode, AppResult},
    indexing::{
        commit::{PageCommitRequest, commit_validated_page},
        state,
        validator::{ValidatedBlock, ValidatedPage},
    },
    maintenance::gate::MaintenanceGate,
};

use super::*;
use crate::learning::{orchestrator::LearningOrchestrator, registry::LearningRequestRegistry};

const FIXTURE_TIME: &str = "2026-08-06T00:00:00.000Z";
const QUESTION: &str = "How does orbital flux determine the synthetic example?";
const TEACHING_SENTINEL: &str = "Teach with one bounded counterexample.";

#[test]
fn new_book_questions_are_deterministic_private_and_local_for_all_formats() {
    let fixture = Fixture::new();
    tauri::async_runtime::block_on(async {
        let books = [
            seed_book(fixture.pool(), BookFormat::Pdf, "pdf").await,
            seed_book(fixture.pool(), BookFormat::Epub, "epub").await,
            seed_book(fixture.pool(), BookFormat::Docx, "docx").await,
        ];
        let decoy = seed_book(fixture.pool(), BookFormat::Pdf, "decoy").await;
        let before = protected_counts(fixture.pool()).await;

        for book in &books {
            fixture.access.reset_counts();
            let metadata = PrepareBookLearningRequestMetadata::New {
                book_id: book.book_id,
                question: QUESTION.to_owned(),
            };
            let metadata_debug = format!("{metadata:?}");
            assert!(!metadata_debug.contains(QUESTION));
            assert!(!metadata_debug.contains(&book.book_id.to_string()));

            let first = fixture.service.prepare(metadata.clone()).await.unwrap();
            assert!(!first.requires_blocking_confirmation);
            assert!(first.source_count >= 2);
            assert_eq!(first.citation_count, 1);
            let prepared = fixture
                .service
                .consume_for_execution(first.preparation_id)
                .await
                .unwrap();
            assert_eq!(prepared.book_id(), book.book_id);
            assert_eq!(prepared.conversation_scope(), ConversationScope::Book);
            assert_eq!(prepared.conversation_id(), None);
            assert_eq!(prepared.expected_next_ordinal(), None);
            assert_eq!(prepared.current_question().unwrap(), QUESTION);
            assert_eq!(
                prepared.teaching_instruction().instruction,
                TEACHING_SENTINEL
            );
            assert_eq!(prepared.provider_profile_id(), fixture.profile_id);
            assert_eq!(prepared.model_id(), "gpt-5.6");

            let request = prepared.chat_request();
            assert_eq!(request.expected_language.as_deref(), Some("zh-CN"));
            assert!(request.system.contains("Response language code: \"zh-CN\""));
            assert_eq!(request.messages.len(), 1);
            assert_eq!(request.messages[0].role, UnifiedRole::User);
            assert_eq!(request.messages[0].content, QUESTION);
            assert!(request.system.contains(TEACHING_SENTINEL));
            assert!(request.system.contains(&book.outline_sentinel));
            assert!(request.system.contains(&book.search_sentinel));
            assert!(request.system.contains("\"source\":\"directory\""));
            assert!(
                request
                    .system
                    .contains("table_of_contents_metadata_not_textbook_source")
            );
            assert!(request.system.contains("\"source\":\"local_text\""));
            assert!(request.system.contains("TL-C1"));
            assert!(!request.system.contains(&book.whole_book_sentinel));
            assert!(!request.system.contains(&decoy.search_sentinel));
            assert!(!request.system.contains(&book.stored_path));

            let context = prepared.packed_context();
            assert!(context.segments.iter().any(|segment| {
                segment.source == ContextSource::Directory
                    && segment.content == book.outline_sentinel
                    && segment.citation.is_none()
            }));
            let local = context
                .segments
                .iter()
                .find(|segment| segment.content == book.search_sentinel)
                .unwrap();
            assert_eq!(local.source, ContextSource::LocalText);
            assert_eq!(local.citation.as_ref().unwrap().id, "TL-C1");
            assert_eq!(context.citations.len(), 1);
            assert_eq!(context.citations[0].book_id, book.book_id);
            assert_eq!(context.citations[0].section_id, Some(book.section_id));
            assert_eq!(context.citations[0].source, DurableContentSource::LocalText);
            assert!(context.citations[0].quoteable);

            let prepared_debug = format!("{prepared:?}");
            for secret in [
                QUESTION,
                TEACHING_SENTINEL,
                book.outline_sentinel.as_str(),
                book.search_sentinel.as_str(),
                book.stored_path.as_str(),
                prepared.model_id(),
            ] {
                assert!(!prepared_debug.contains(secret));
            }

            let second = fixture.service.prepare(metadata).await.unwrap();
            let repeated = fixture
                .service
                .consume_for_execution(second.preparation_id)
                .await
                .unwrap();
            assert_eq!(request, repeated.chat_request());
            assert_eq!(context.segments, repeated.packed_context().segments);
            assert_eq!(context.citations, repeated.packed_context().citations);
            assert_eq!(
                context.estimated_input_tokens,
                repeated.packed_context().estimated_input_tokens
            );
            assert!(fixture.access.credential_calls() >= 4);
            assert!(fixture.access.consent_calls() >= 4);
            assert!(fixture.access.saw_learning_lease());
        }

        assert_eq!(before, protected_counts(fixture.pool()).await);
        assert_eq!(
            fixture.gate.status().code,
            MaintenanceStatusCode::MaintenanceAvailable
        );
    });
}

#[test]
fn consumed_book_request_registers_before_provider_load_failure_and_persists_nothing() {
    let fixture = Fixture::new();
    tauri::async_runtime::block_on(async {
        let book = seed_book(fixture.pool(), BookFormat::Pdf, "provider-load-failure").await;
        let summary = fixture
            .service
            .prepare(new_metadata(book.book_id, QUESTION))
            .await
            .unwrap();
        let prepared = fixture
            .service
            .consume_for_execution(summary.preparation_id)
            .await
            .unwrap();
        assert_eq!(
            fixture
                .service
                .consume_for_execution(summary.preparation_id)
                .await
                .unwrap_err()
                .code,
            AppErrorCode::RequestConflict,
            "opaque preparation is consumed exactly once"
        );

        let registry = LearningRequestRegistry::with_maintenance_gate(fixture.gate.clone());
        let orchestrator = LearningOrchestrator::new(registry.clone(), fixture.pool().clone());
        let runtime_store = Arc::new(MissingRuntimeCredentialStore {
            gate: fixture.gate.clone(),
            saw_learning_lease: AtomicBool::new(false),
        });
        let runtime = ProviderRuntime::new(
            runtime_store.clone(),
            ProviderCapabilityRegistry::load_embedded().unwrap(),
        );
        let permit = registry.acquire_maintenance_permit().unwrap();
        let snapshot = orchestrator
            .start_prepared_book_with_maintenance_permit(&runtime, prepared, permit)
            .await
            .unwrap();
        assert_eq!(snapshot.status, LearningRequestStatus::Failed);
        assert_eq!(snapshot.conversation_id, None);
        assert!(snapshot.safe_error.is_some());
        assert!(runtime_store.saw_learning_lease.load(Ordering::SeqCst));
        assert_eq!(
            registry.snapshot(snapshot.request_id).unwrap().status,
            LearningRequestStatus::Failed
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM conversations")
                .fetch_one(fixture.pool())
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM messages")
                .fetch_one(fixture.pool())
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM annotations")
                .fetch_one(fixture.pool())
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            fixture.gate.status().code,
            MaintenanceStatusCode::MaintenanceAvailable
        );
    });
}

#[test]
fn command_boundary_permit_allows_consume_to_finish_ahead_of_queued_maintenance() {
    let fixture = Fixture::new();
    tauri::async_runtime::block_on(async {
        let book = seed_book(fixture.pool(), BookFormat::Pdf, "queued-maintenance").await;
        let summary = fixture
            .service
            .prepare(new_metadata(book.book_id, QUESTION))
            .await
            .unwrap();
        let command_permit = fixture
            .gate
            .try_acquire_normal(ActiveOperationKind::Learning)
            .unwrap();
        let queued_gate = fixture.gate.clone();
        let queued = tokio::spawn(async move {
            let _exclusive = queued_gate.acquire_maintenance().await.unwrap();
        });
        for _ in 0..100 {
            if fixture.gate.status().code == MaintenanceStatusCode::MaintenanceWaiting {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert_eq!(
            fixture.gate.status().code,
            MaintenanceStatusCode::MaintenanceWaiting
        );

        let prepared = fixture
            .service
            .consume_for_execution_with_maintenance_permit(summary.preparation_id, &command_permit)
            .await
            .unwrap();
        assert_eq!(prepared.book_id(), book.book_id);
        drop(prepared);
        drop(command_permit);
        queued.await.unwrap();
        assert_eq!(
            fixture.gate.status().code,
            MaintenanceStatusCode::MaintenanceAvailable
        );
    });
}

#[test]
fn provider_load_continuation_keeps_maintenance_lease_after_registry_shutdown() {
    let fixture = Fixture::new();
    tauri::async_runtime::block_on(async {
        let book = seed_book(fixture.pool(), BookFormat::Pdf, "shutdown-provider-load").await;
        let summary = fixture
            .service
            .prepare(new_metadata(book.book_id, QUESTION))
            .await
            .unwrap();
        let prepared = fixture
            .service
            .consume_for_execution(summary.preparation_id)
            .await
            .unwrap();

        let registry = LearningRequestRegistry::with_maintenance_gate(fixture.gate.clone());
        let orchestrator = LearningOrchestrator::new(registry.clone(), fixture.pool().clone());
        let (reached_sender, reached_receiver) = oneshot::channel();
        let runtime_store = Arc::new(BlockingRuntimeCredentialStore {
            reached: Mutex::new(Some(reached_sender)),
            release: Semaphore::new(0),
        });
        let runtime = ProviderRuntime::new(
            runtime_store.clone(),
            ProviderCapabilityRegistry::load_embedded().unwrap(),
        );
        let permit = registry.acquire_maintenance_permit().unwrap();
        let start = tokio::spawn(async move {
            orchestrator
                .start_prepared_book_with_maintenance_permit(&runtime, prepared, permit)
                .await
        });
        reached_receiver.await.unwrap();
        registry.shutdown();

        let error = fixture.gate.try_acquire_maintenance().unwrap_err();
        assert!(matches!(
            error,
            crate::maintenance::gate::GateAcquireError::Busy(_)
        ));
        runtime_store.release.add_permits(1);
        assert_eq!(
            start.await.unwrap().unwrap_err().code,
            AppErrorCode::NotFound
        );
        let exclusive = fixture.gate.try_acquire_maintenance().unwrap();
        drop(exclusive);
        assert_eq!(
            fixture.gate.status().code,
            MaintenanceStatusCode::MaintenanceAvailable
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM conversations")
                .fetch_one(fixture.pool())
                .await
                .unwrap(),
            0
        );
    });
}

#[test]
fn book_execution_rejects_provider_model_drift_after_consume_without_persistence() {
    let fixture = Fixture::new();
    tauri::async_runtime::block_on(async {
        let book = seed_book(fixture.pool(), BookFormat::Pdf, "provider-drift").await;
        let summary = fixture
            .service
            .prepare(new_metadata(book.book_id, QUESTION))
            .await
            .unwrap();
        let prepared = fixture
            .service
            .consume_for_execution(summary.preparation_id)
            .await
            .unwrap();

        sqlx::query(
            "UPDATE provider_profiles SET provider_kind = 'gemini', model_id = 'gemini-3.6-flash', updated_at = '2026-08-06T00:04:00.000Z' WHERE id = ?",
        )
        .bind(fixture.profile_id.to_string())
        .execute(fixture.pool())
        .await
        .unwrap();
        let credentials = Arc::new(MemoryCredentialStore::new());
        credentials
            .set(
                &providers::credential_key(fixture.profile_id),
                SecretString::from("synthetic-not-a-real-key"),
            )
            .await
            .unwrap();
        let runtime = ProviderRuntime::new(
            credentials,
            ProviderCapabilityRegistry::load_embedded().unwrap(),
        );
        let registry = LearningRequestRegistry::with_maintenance_gate(fixture.gate.clone());
        let orchestrator = LearningOrchestrator::new(registry.clone(), fixture.pool().clone());
        let permit = registry.acquire_maintenance_permit().unwrap();
        let snapshot = orchestrator
            .start_prepared_book_with_maintenance_permit(&runtime, prepared, permit)
            .await
            .unwrap();
        assert_eq!(snapshot.status, LearningRequestStatus::Failed);
        assert_eq!(
            snapshot.safe_error.unwrap().code,
            AppError::new(AppErrorCode::RequestConflict).stable_code()
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM conversations")
                .fetch_one(fixture.pool())
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM messages")
                .fetch_one(fixture.pool())
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            fixture.gate.status().code,
            MaintenanceStatusCode::MaintenanceAvailable
        );
    });
}

#[test]
fn continue_uses_only_complete_same_book_conversation_pairs() {
    let fixture = Fixture::new();
    tauri::async_runtime::block_on(async {
        let target = seed_book(fixture.pool(), BookFormat::Pdf, "target").await;
        let decoy = seed_book(fixture.pool(), BookFormat::Pdf, "other").await;
        let target_conversation = seed_book_conversation(
            fixture.pool(),
            target.book_id,
            fixture.profile_id,
            &[
                ("TARGET_USER_0", "TARGET_ASSISTANT_1 cites TL-C9 safely"),
                ("TARGET_USER_2", "TARGET_ASSISTANT_3"),
            ],
        )
        .await;
        let other_same_book = seed_book_conversation(
            fixture.pool(),
            target.book_id,
            fixture.profile_id,
            &[("SAME_BOOK_DECOY_USER", "SAME_BOOK_DECOY_ASSISTANT")],
        )
        .await;
        let other_book_conversation = seed_book_conversation(
            fixture.pool(),
            decoy.book_id,
            fixture.profile_id,
            &[("OTHER_BOOK_USER", "OTHER_BOOK_ASSISTANT")],
        )
        .await;
        let selection_conversation =
            seed_selection_conversation(fixture.pool(), &target, fixture.profile_id).await;

        let summary = fixture
            .service
            .prepare(PrepareBookLearningRequestMetadata::Continue {
                book_id: target.book_id,
                conversation_id: target_conversation,
                question: "What should I connect next?".to_owned(),
            })
            .await
            .unwrap();
        let prepared = fixture
            .service
            .consume_for_execution(summary.preparation_id)
            .await
            .unwrap();
        assert_eq!(prepared.conversation_id(), Some(target_conversation));
        assert_eq!(prepared.expected_next_ordinal(), Some(4));
        let messages = &prepared.chat_request().messages;
        assert_eq!(messages.len(), 5);
        assert_eq!(messages[0].content, "TARGET_USER_0");
        assert_eq!(
            messages[1].content,
            "TARGET_ASSISTANT_1 cites [historical citation omitted] safely"
        );
        assert_eq!(messages[2].content, "TARGET_USER_2");
        assert_eq!(messages[3].content, "TARGET_ASSISTANT_3");
        assert_eq!(messages[4].content, "What should I connect next?");
        let rendered = format!("{}\n{:?}", prepared.chat_request().system, messages);
        for forbidden in [
            "SAME_BOOK_DECOY_USER",
            "SAME_BOOK_DECOY_ASSISTANT",
            "OTHER_BOOK_USER",
            "OTHER_BOOK_ASSISTANT",
        ] {
            assert!(!rendered.contains(forbidden));
        }

        for (book_id, conversation_id, expected) in [
            (decoy.book_id, target_conversation, AppErrorCode::NotFound),
            (
                target.book_id,
                selection_conversation,
                AppErrorCode::InvalidInput,
            ),
            (
                target.book_id,
                other_book_conversation,
                AppErrorCode::NotFound,
            ),
        ] {
            let error = fixture
                .service
                .prepare(PrepareBookLearningRequestMetadata::Continue {
                    book_id,
                    conversation_id,
                    question: "Bounded follow-up?".to_owned(),
                })
                .await
                .unwrap_err();
            assert_eq!(error.code, expected);
        }

        sqlx::query("UPDATE messages SET action = 'ask' WHERE conversation_id = ? AND ordinal = 2")
            .bind(target_conversation.to_string())
            .execute(fixture.pool())
            .await
            .unwrap();
        fixture.access.reset_counts();
        let corrupted = fixture
            .service
            .prepare(PrepareBookLearningRequestMetadata::Continue {
                book_id: target.book_id,
                conversation_id: target_conversation,
                question: "Does corruption fail closed?".to_owned(),
            })
            .await
            .unwrap_err();
        assert_eq!(corrupted.code, AppErrorCode::DatabaseError);
        assert_eq!(fixture.access.credential_calls(), 0);
        assert_eq!(fixture.access.consent_calls(), 0);

        assert_ne!(other_same_book, target_conversation);
    });
}

#[test]
fn invalid_questions_capability_and_overflow_fail_before_sensitive_access() {
    let fixture = Fixture::new();
    tauri::async_runtime::block_on(async {
        let book = seed_book(fixture.pool(), BookFormat::Pdf, "validation").await;
        for question in [
            String::new(),
            "   \n\t".to_owned(),
            "invalid\u{0001}control".to_owned(),
            "界".repeat(crate::db::messages::MAX_LEARNING_QUESTION_CODE_POINTS + 1),
        ] {
            fixture.access.reset_counts();
            let error = fixture
                .service
                .prepare(PrepareBookLearningRequestMetadata::New {
                    book_id: book.book_id,
                    question,
                })
                .await
                .unwrap_err();
            assert_eq!(error.code, AppErrorCode::InvalidInput);
            assert_eq!(fixture.access.credential_calls(), 0);
            assert_eq!(fixture.access.consent_calls(), 0);
        }

        sqlx::query("UPDATE provider_profiles SET context_window_tokens = 4096 WHERE id = ?")
            .bind(fixture.profile_id.to_string())
            .execute(fixture.pool())
            .await
            .unwrap();
        fixture.access.reset_counts();
        let overflow = fixture
            .service
            .prepare(PrepareBookLearningRequestMetadata::New {
                book_id: book.book_id,
                question: QUESTION.to_owned(),
            })
            .await
            .unwrap_err();
        assert_eq!(overflow.code, AppErrorCode::ContextTooLarge);
        assert_eq!(fixture.access.credential_calls(), 0);
        assert_eq!(fixture.access.consent_calls(), 0);

        sqlx::query(
            "UPDATE provider_profiles SET context_window_tokens = 128000, model_id = 'unknown-text-model', updated_at = '2026-08-06T00:01:00.000Z' WHERE id = ?",
        )
        .bind(fixture.profile_id.to_string())
        .execute(fixture.pool())
        .await
        .unwrap();
        fixture.access.reset_counts();
        let unsupported = fixture
            .service
            .prepare(PrepareBookLearningRequestMetadata::New {
                book_id: book.book_id,
                question: QUESTION.to_owned(),
            })
            .await
            .unwrap_err();
        assert_eq!(unsupported.code, AppErrorCode::InvalidInput);
        assert_eq!(unsupported.stable_code(), "UNSUPPORTED_PROVIDER_CAPABILITY");
        assert_eq!(fixture.access.credential_calls(), 0);
        assert_eq!(fixture.access.consent_calls(), 0);
    });
}

#[test]
fn outline_history_and_retrieval_have_independent_caps_and_cost_authorization() {
    let fixture = Fixture::new();
    tauri::async_runtime::block_on(async {
        let book = seed_book(fixture.pool(), BookFormat::Pdf, "bounded").await;
        add_outline_sections(fixture.pool(), &book, 270).await;
        let pairs = (0..20)
            .map(|index| {
                (
                    format!("BOUNDED_HISTORY_USER_{index}"),
                    format!("BOUNDED_HISTORY_ASSISTANT_{index}"),
                )
            })
            .collect::<Vec<_>>();
        let borrowed = pairs
            .iter()
            .map(|(user, assistant)| (user.as_str(), assistant.as_str()))
            .collect::<Vec<_>>();
        let conversation_id =
            seed_book_conversation(fixture.pool(), book.book_id, fixture.profile_id, &borrowed)
                .await;
        let long_retrieval = format!("heliopause {}", "x".repeat(40_000));
        sqlx::query("UPDATE search_chunks SET text = ? WHERE book_id = ?")
            .bind(&long_retrieval)
            .bind(book.book_id.to_string())
            .execute(fixture.pool())
            .await
            .unwrap();
        sqlx::query("UPDATE app_settings SET context_mode = 'long' WHERE id = 1")
            .execute(fixture.pool())
            .await
            .unwrap();
        fixture
            .access
            .set_cost_decision(ProviderOperationConsentDecision::Ask);

        let summary = fixture
            .service
            .prepare(PrepareBookLearningRequestMetadata::Continue {
                book_id: book.book_id,
                conversation_id,
                question: "Explain heliopause with the recent discussion.".to_owned(),
            })
            .await
            .unwrap();
        assert!(summary.estimated_input_tokens > STANDARD_CONTEXT_CAP);
        assert_eq!(
            summary.risk_flags,
            vec![BookLearningPreparationRiskFlag::CostRisk]
        );
        assert!(summary.requires_blocking_confirmation);
        assert!(summary.omitted_source_count >= 14);
        let blocked = fixture
            .service
            .consume_for_execution(summary.preparation_id)
            .await
            .unwrap_err();
        assert_eq!(blocked.code, AppErrorCode::RequestConflict);

        fixture
            .service
            .authorize(summary.preparation_id, LearningAuthorizationDecision::Allow)
            .await
            .unwrap();
        let prepared = fixture
            .service
            .consume_for_execution(summary.preparation_id)
            .await
            .unwrap();
        let context = prepared.packed_context();
        assert_eq!(
            context
                .segments
                .iter()
                .filter(|segment| segment.source == ContextSource::Directory)
                .count(),
            MAX_OUTLINE_ITEMS
        );
        assert!(context.segments.iter().any(|segment| {
            segment.source == ContextSource::HistorySummary
                && segment.content == crate::retrieval::context::HISTORY_PLACEHOLDER
                && segment.citation.is_none()
        }));
        assert!(context.segments.iter().any(|segment| {
            segment.source == ContextSource::LocalText && segment.content == long_retrieval
        }));
        let messages = &prepared.chat_request().messages;
        assert_eq!(messages.len(), MAX_HISTORY_MESSAGES + 1);
        assert!(
            !messages
                .iter()
                .any(|message| message.content == "BOUNDED_HISTORY_USER_0")
        );
        assert!(
            messages
                .iter()
                .any(|message| message.content == "BOUNDED_HISTORY_USER_4")
        );
        assert!(
            messages
                .iter()
                .any(|message| message.content == "BOUNDED_HISTORY_ASSISTANT_19")
        );
        assert_eq!(
            messages.last().unwrap().content,
            "Explain heliopause with the recent discussion."
        );
    });
}

#[test]
fn retrieval_dropped_before_history_is_not_reintroduced_after_history_compression() {
    let book_id = Uuid::new_v4();
    let section_id = stable_section_id(book_id, 0);
    let timestamp = Utc.with_ymd_and_hms(2026, 8, 6, 0, 0, 0).single().unwrap();
    let snapshot = LocalPreparationSnapshot {
        binding: ProviderBindingSnapshot {
            book: BookBindingSnapshot {
                book_id,
                format: BookFormat::Pdf,
                source_sha256: "a".repeat(64),
                updated_at: FIXTURE_TIME.to_owned(),
            },
            profile_id: Uuid::new_v4(),
            provider_kind: ProviderKind::OpenAi,
            provider_display_name: "Synthetic provider".to_owned(),
            profile_display_name: "Synthetic profile".to_owned(),
            model_id: "gpt-5.6".to_owned(),
            model_display_name: "Synthetic model".to_owned(),
            context_window_tokens: 10_000,
            default_max_output_tokens: 4_096,
            validated_at: timestamp,
            profile_updated_at: FIXTURE_TIME.to_owned(),
            context_mode: ContextMode::Standard,
            ui_language: UiLanguage::ZhCn,
        },
        teaching_instruction: TeachingInstructionDto {
            instruction: String::new(),
            revision: 0,
            updated_at: timestamp,
        },
        target: BookPreparationTarget::Continue {
            conversation_id: Uuid::new_v4(),
            expected_next_ordinal: 2,
        },
        outline: vec![ContextCandidate {
            stable_id: format!("directory:{section_id}"),
            book_id,
            section_id: Some(section_id),
            source_kind: ContextSourceKind::Directory,
            source: ContextSource::Directory,
            same_section: false,
            relevance_micros: 0,
            ordinal: 0,
            text: "Bounded chapter".to_owned(),
            locator_label: "table of contents item 1".to_owned(),
            locator: None,
            review_status: CitationReviewStatus::NotRequired,
            provenance_key: "directory:0".to_owned(),
            citation_seed: None,
        }],
        retrieval: vec![ContextCandidate {
            stable_id: "retrieval:low-priority".to_owned(),
            book_id,
            section_id: None,
            source_kind: ContextSourceKind::TextbookSearch,
            source: ContextSource::AiDescription,
            same_section: false,
            relevance_micros: 1,
            ordinal: 0,
            text: "R".repeat(1_000),
            locator_label: "page 1".to_owned(),
            locator: Some(DocumentLocator::Pdf {
                start_page: 1,
                end_page: 1,
                rects_by_page: None,
            }),
            review_status: CitationReviewStatus::NotRequired,
            provenance_key: "ai-description:page-1".to_owned(),
            citation_seed: None,
        }],
        history: vec![
            UnifiedMessage {
                role: UnifiedRole::User,
                content: "U".repeat(4_000),
            },
            UnifiedMessage {
                role: UnifiedRole::Assistant,
                content: "A".repeat(4_000),
            },
        ],
        total_history_messages: 2,
        omitted_outline_items: 0,
        omitted_retrieval_items: 0,
        history_was_omitted: false,
        fingerprint: SnapshotFingerprint([0; 32]),
    };

    let (prompt, packed, retained_history) =
        pack_local_snapshot(&snapshot, "What is the bounded result?").unwrap();

    assert_eq!(retained_history, 0);
    assert_eq!(prompt.messages.len(), 1);
    assert_eq!(packed.omitted_segment_count, 1);
    assert!(
        !packed
            .segments
            .iter()
            .any(|segment| segment.source_kind == ContextSourceKind::TextbookSearch)
    );
    assert!(
        packed
            .segments
            .iter()
            .any(|segment| segment.source == ContextSource::HistorySummary)
    );
}

#[test]
fn snapshot_races_deletions_and_corrupt_ownership_fail_closed() {
    let fixture = Fixture::new();
    tauri::async_runtime::block_on(async {
        let book = seed_book(fixture.pool(), BookFormat::Pdf, "race").await;
        fixture.access.mutate_teaching_on_credential();
        let raced = fixture
            .service
            .prepare(PrepareBookLearningRequestMetadata::New {
                book_id: book.book_id,
                question: QUESTION.to_owned(),
            })
            .await
            .unwrap_err();
        assert_eq!(raced.code, AppErrorCode::RequestConflict);
        assert_eq!(fixture.registry.stats().0, 0);

        fixture.access.reset_mutation();
        let summary = fixture
            .service
            .prepare(PrepareBookLearningRequestMetadata::New {
                book_id: book.book_id,
                question: QUESTION.to_owned(),
            })
            .await
            .unwrap();
        sqlx::query(
            "UPDATE provider_profiles SET updated_at = '2026-08-06T00:02:00.000Z' WHERE id = ?",
        )
        .bind(fixture.profile_id.to_string())
        .execute(fixture.pool())
        .await
        .unwrap();
        let changed = fixture
            .service
            .consume_for_execution(summary.preparation_id)
            .await
            .unwrap_err();
        assert_eq!(changed.code, AppErrorCode::RequestConflict);
        assert_eq!(fixture.registry.stats().0, 0);

        sqlx::query("UPDATE provider_profiles SET updated_at = ? WHERE id = ?")
            .bind(FIXTURE_TIME)
            .bind(fixture.profile_id.to_string())
            .execute(fixture.pool())
            .await
            .unwrap();
        let summary = fixture
            .service
            .prepare(PrepareBookLearningRequestMetadata::New {
                book_id: book.book_id,
                question: QUESTION.to_owned(),
            })
            .await
            .unwrap();
        sqlx::query("UPDATE app_settings SET default_learning_profile_id = NULL WHERE id = 1")
            .execute(fixture.pool())
            .await
            .unwrap();
        let deleted_default = fixture
            .service
            .consume_for_execution(summary.preparation_id)
            .await
            .unwrap_err();
        assert_eq!(deleted_default.code, AppErrorCode::RequestConflict);

        sqlx::query("UPDATE app_settings SET default_learning_profile_id = ? WHERE id = 1")
            .bind(fixture.profile_id.to_string())
            .execute(fixture.pool())
            .await
            .unwrap();
        sqlx::query("UPDATE search_chunks SET section_id = ? WHERE book_id = ?")
            .bind(Uuid::new_v4().to_string())
            .bind(book.book_id.to_string())
            .execute(fixture.pool())
            .await
            .unwrap_err();
        let mut corrupt_connection = fixture.pool().acquire().await.unwrap();
        sqlx::query("PRAGMA foreign_keys = OFF")
            .execute(&mut *corrupt_connection)
            .await
            .unwrap();
        sqlx::query("UPDATE search_chunks SET section_id = ? WHERE book_id = ?")
            .bind(Uuid::new_v4().to_string())
            .bind(book.book_id.to_string())
            .execute(&mut *corrupt_connection)
            .await
            .unwrap();
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&mut *corrupt_connection)
            .await
            .unwrap();
        drop(corrupt_connection);
        fixture.access.reset_counts();
        let corrupt = fixture
            .service
            .prepare(PrepareBookLearningRequestMetadata::New {
                book_id: book.book_id,
                question: QUESTION.to_owned(),
            })
            .await
            .unwrap_err();
        assert_eq!(corrupt.code, AppErrorCode::DatabaseError);
        assert_eq!(fixture.access.credential_calls(), 0);
        assert_eq!(fixture.access.consent_calls(), 0);
    });
}

#[test]
fn registry_is_single_use_ttl_bounded_invalidatable_and_maintenance_gated() {
    assert_eq!(BOOK_PREPARATION_TTL, Duration::from_secs(5 * 60));
    let fixture = Fixture::new();
    tauri::async_runtime::block_on(async {
        let first_book = seed_book(fixture.pool(), BookFormat::Pdf, "registry-a").await;
        let second_book = seed_book(fixture.pool(), BookFormat::Pdf, "registry-b").await;

        let once = fixture
            .service
            .prepare(new_metadata(first_book.book_id, "Single use?"))
            .await
            .unwrap();
        fixture
            .service
            .consume_for_execution(once.preparation_id)
            .await
            .unwrap();
        assert_eq!(
            fixture
                .service
                .consume_for_execution(once.preparation_id)
                .await
                .unwrap_err()
                .code,
            AppErrorCode::RequestConflict
        );

        let expired = fixture
            .service
            .prepare(new_metadata(first_book.book_id, "Expire me?"))
            .await
            .unwrap();
        fixture
            .registry
            .set_expiry_for_test(
                expired.preparation_id,
                Instant::now() - Duration::from_secs(1),
            )
            .unwrap();
        assert_eq!(fixture.registry.garbage_collect(), 1);
        assert_eq!(fixture.registry.stats().0, 0);

        let first = fixture
            .service
            .prepare(new_metadata(first_book.book_id, "Invalidate first?"))
            .await
            .unwrap();
        let second = fixture
            .service
            .prepare(new_metadata(second_book.book_id, "Keep second?"))
            .await
            .unwrap();
        assert_eq!(
            fixture
                .registry
                .invalidate(&InvalidateLearningPreparations {
                    reason: LearningInvalidationReason::BookChange,
                    book_id: Some(first_book.book_id),
                    provider_profile_id: None,
                })
                .unwrap(),
            1
        );
        assert_eq!(
            fixture
                .service
                .consume_for_execution(first.preparation_id)
                .await
                .unwrap_err()
                .code,
            AppErrorCode::RequestConflict
        );
        fixture
            .service
            .consume_for_execution(second.preparation_id)
            .await
            .unwrap();

        let exclusive = fixture.gate.try_acquire_maintenance().unwrap();
        fixture.access.reset_counts();
        let gated = fixture
            .service
            .prepare(new_metadata(first_book.book_id, "Maintenance conflict?"))
            .await
            .unwrap_err();
        assert_eq!(gated.code, AppErrorCode::RequestConflict);
        assert_eq!(fixture.access.credential_calls(), 0);
        assert_eq!(fixture.access.consent_calls(), 0);
        drop(exclusive);

        let source = include_str!("book_preparation.rs");
        for forbidden in [
            "ProviderRuntime",
            "stream_text_learning",
            "reqwest",
            "INSERT INTO ",
            "UPDATE ",
            "DELETE FROM ",
        ] {
            assert!(
                !source.contains(forbidden),
                "forbidden prepare dependency: {forbidden}"
            );
        }
    });
}

#[test]
fn indexed_sources_and_correction_overlay_keep_quoteability_distinct() {
    let fixture = Fixture::new();
    tauri::async_runtime::block_on(async {
        let book = seed_book(fixture.pool(), BookFormat::Pdf, "overlay").await;
        let (page_id, block_id) = commit_indexed_page(
            fixture.pool(),
            book.book_id,
            fixture.profile_id,
            "Quantum lattice provider original",
            "Quantum lattice visual description",
        )
        .await;

        let original = fixture
            .service
            .prepare(new_metadata(
                book.book_id,
                "Explain the quantum lattice evidence.",
            ))
            .await
            .unwrap();
        let original = fixture
            .service
            .consume_for_execution(original.preparation_id)
            .await
            .unwrap();
        let transcribed = original
            .packed_context()
            .segments
            .iter()
            .find(|segment| segment.source == ContextSource::AiTranscribed)
            .unwrap();
        assert_eq!(transcribed.content, "Quantum lattice provider original");
        assert_eq!(transcribed.review_status, CitationReviewStatus::Indexed);
        assert!(transcribed.citation.is_some());
        let description = original
            .packed_context()
            .segments
            .iter()
            .find(|segment| segment.source == ContextSource::AiDescription)
            .unwrap();
        assert_eq!(description.content, "Quantum lattice visual description");
        assert!(description.citation.is_none());

        let correction = save_index_correction(
            fixture.pool(),
            SaveIndexCorrection {
                book_id: book.book_id,
                page_id,
                target_block_id: block_id,
                target_content_version: 1,
                value_kind: IndexCorrectionValueKind::Text,
                original_value_sha256: correction_value_sha256("Quantum lattice provider original"),
                corrected_value: "Quantum lattice accepted correction".to_owned(),
                expected_revision: 0,
            },
        )
        .await
        .unwrap();
        let accepted = fixture
            .service
            .prepare(new_metadata(
                book.book_id,
                "Explain the accepted quantum lattice correction.",
            ))
            .await
            .unwrap();
        let accepted = fixture
            .service
            .consume_for_execution(accepted.preparation_id)
            .await
            .unwrap();
        let corrected = accepted
            .packed_context()
            .segments
            .iter()
            .find(|segment| segment.source == ContextSource::UserCorrected)
            .unwrap();
        assert_eq!(corrected.content, "Quantum lattice accepted correction");
        assert_eq!(corrected.review_status, CitationReviewStatus::UserCorrected);
        assert!(corrected.citation.is_some());
        assert!(!accepted.packed_context().segments.iter().any(|segment| {
            segment.source == ContextSource::AiTranscribed
                && segment.content == "Quantum lattice provider original"
        }));
        let rendered = &accepted.chat_request().system;
        assert!(!rendered.contains(&correction.id.to_string()));
        assert!(!rendered.contains(&page_id.to_string()));
        assert!(!rendered.contains(&block_id.to_string()));

        sqlx::query(
            "UPDATE index_corrections SET conflict_state = 'conflict', updated_at = ? WHERE id = ?",
        )
        .bind(FIXTURE_TIME)
        .bind(correction.id.to_string())
        .execute(fixture.pool())
        .await
        .unwrap();
        let conflicted = fixture
            .service
            .prepare(new_metadata(
                book.book_id,
                "Explain the quantum lattice after conflict.",
            ))
            .await
            .unwrap();
        let conflicted = fixture
            .service
            .consume_for_execution(conflicted.preparation_id)
            .await
            .unwrap();
        assert!(!conflicted.packed_context().segments.iter().any(|segment| {
            matches!(
                segment.source,
                ContextSource::AiTranscribed | ContextSource::UserCorrected
            )
        }));
        assert!(conflicted.packed_context().segments.iter().any(|segment| {
            segment.source == ContextSource::AiDescription && segment.citation.is_none()
        }));
    });
}

fn new_metadata(book_id: Uuid, question: &str) -> PrepareBookLearningRequestMetadata {
    PrepareBookLearningRequestMetadata::New {
        book_id,
        question: question.to_owned(),
    }
}

struct Fixture {
    _temporary: tempfile::TempDir,
    database: Database,
    profile_id: Uuid,
    registry: BookPreparationRegistry,
    access: Arc<CountingBookSensitiveAccess>,
    gate: MaintenanceGate,
    service: BookLearningPreparationService,
}

impl Fixture {
    fn new() -> Self {
        let temporary = tempfile::tempdir().unwrap();
        let database = Database::open(temporary.path().join("book-preparation.sqlite3")).unwrap();
        let profile_id = Uuid::new_v4();
        tauri::async_runtime::block_on(seed_profile(database.pool(), profile_id));
        let registry = BookPreparationRegistry::default();
        let gate = MaintenanceGate::default();
        let access = Arc::new(CountingBookSensitiveAccess::new(
            database.pool().clone(),
            profile_id,
            gate.clone(),
        ));
        let service = BookLearningPreparationService::new(
            database.pool().clone(),
            registry.clone(),
            ProviderCapabilityRegistry::load_embedded().unwrap(),
            Arc::new(MemoryCredentialStore::new()),
            gate.clone(),
        )
        .with_sensitive_access(access.clone());
        Self {
            _temporary: temporary,
            database,
            profile_id,
            registry,
            access,
            gate,
            service,
        }
    }

    fn pool(&self) -> &SqlitePool {
        self.database.pool()
    }
}

struct CountingBookSensitiveAccess {
    pool: SqlitePool,
    profile_id: Uuid,
    gate: MaintenanceGate,
    credential_calls: AtomicUsize,
    consent_calls: AtomicUsize,
    saw_learning_lease: AtomicBool,
    mutate_teaching: AtomicBool,
    consents: Mutex<Vec<ProviderOperationConsent>>,
}

struct MissingRuntimeCredentialStore {
    gate: MaintenanceGate,
    saw_learning_lease: AtomicBool,
}

struct BlockingRuntimeCredentialStore {
    reached: Mutex<Option<oneshot::Sender<()>>>,
    release: Semaphore,
}

#[async_trait]
impl CredentialStore for BlockingRuntimeCredentialStore {
    async fn set(&self, _key: &str, _value: SecretString) -> AppResult<()> {
        Err(AppError::credential_store(
            "synthetic runtime store is read-only",
        ))
    }

    async fn get(&self, _key: &str) -> AppResult<SecretString> {
        if let Some(reached) = self.reached.lock().take() {
            let _ = reached.send(());
        }
        self.release
            .acquire()
            .await
            .map_err(|_| AppError::credential_store("synthetic runtime store closed"))?
            .forget();
        Ok(SecretString::from("synthetic-not-a-real-key"))
    }

    async fn delete(&self, _key: &str) -> AppResult<()> {
        Err(AppError::credential_store(
            "synthetic runtime store is read-only",
        ))
    }
}

#[async_trait]
impl CredentialStore for MissingRuntimeCredentialStore {
    async fn set(&self, _key: &str, _value: SecretString) -> AppResult<()> {
        Err(AppError::credential_store(
            "synthetic runtime store is read-only",
        ))
    }

    async fn get(&self, _key: &str) -> AppResult<SecretString> {
        if self
            .gate
            .status()
            .active_operations
            .iter()
            .any(|operation| {
                operation.kind == ActiveOperationKind::Learning && operation.count >= 1
            })
        {
            self.saw_learning_lease.store(true, Ordering::SeqCst);
        }
        Err(AppError::credential_store(
            "synthetic runtime credential is missing",
        ))
    }

    async fn delete(&self, _key: &str) -> AppResult<()> {
        Err(AppError::credential_store(
            "synthetic runtime store is read-only",
        ))
    }
}

impl CountingBookSensitiveAccess {
    fn new(pool: SqlitePool, profile_id: Uuid, gate: MaintenanceGate) -> Self {
        Self {
            pool,
            profile_id,
            gate,
            credential_calls: AtomicUsize::new(0),
            consent_calls: AtomicUsize::new(0),
            saw_learning_lease: AtomicBool::new(false),
            mutate_teaching: AtomicBool::new(false),
            consents: Mutex::new(default_consents(profile_id)),
        }
    }

    fn credential_calls(&self) -> usize {
        self.credential_calls.load(Ordering::SeqCst)
    }

    fn consent_calls(&self) -> usize {
        self.consent_calls.load(Ordering::SeqCst)
    }

    fn saw_learning_lease(&self) -> bool {
        self.saw_learning_lease.load(Ordering::SeqCst)
    }

    fn reset_counts(&self) {
        self.credential_calls.store(0, Ordering::SeqCst);
        self.consent_calls.store(0, Ordering::SeqCst);
        self.saw_learning_lease.store(false, Ordering::SeqCst);
    }

    fn set_cost_decision(&self, decision: ProviderOperationConsentDecision) {
        let mut consents = self.consents.lock();
        consents
            .iter_mut()
            .find(|consent| consent.category == ProviderOperationConsentCategory::CostRisk)
            .unwrap()
            .decision = decision;
    }

    fn mutate_teaching_on_credential(&self) {
        self.mutate_teaching.store(true, Ordering::SeqCst);
    }

    fn reset_mutation(&self) {
        self.mutate_teaching.store(false, Ordering::SeqCst);
    }

    fn observe_gate(&self) {
        if self
            .gate
            .status()
            .active_operations
            .iter()
            .any(|operation| {
                operation.kind == ActiveOperationKind::Learning && operation.count == 1
            })
        {
            self.saw_learning_lease.store(true, Ordering::SeqCst);
        }
    }
}

#[async_trait]
impl BookPreparationSensitiveAccess for CountingBookSensitiveAccess {
    async fn require_credential(&self, profile_id: Uuid) -> AppResult<()> {
        self.credential_calls.fetch_add(1, Ordering::SeqCst);
        self.observe_gate();
        if profile_id != self.profile_id {
            return Err(AppError::new(AppErrorCode::RequestConflict));
        }
        if self.mutate_teaching.swap(false, Ordering::SeqCst) {
            sqlx::query(
                "UPDATE teaching_preferences SET instruction = 'RACE_MUTATION_SENTINEL', revision = revision + 1, updated_at = '2026-08-06T00:03:00.000Z' WHERE id = 1",
            )
            .execute(&self.pool)
            .await?;
        }
        Ok(())
    }

    async fn load_consents(
        &self,
        _pool: &SqlitePool,
        profile_id: Uuid,
    ) -> AppResult<Vec<ProviderOperationConsent>> {
        self.consent_calls.fetch_add(1, Ordering::SeqCst);
        self.observe_gate();
        if profile_id != self.profile_id {
            return Err(AppError::new(AppErrorCode::RequestConflict));
        }
        Ok(self.consents.lock().clone())
    }
}

fn default_consents(profile_id: Uuid) -> Vec<ProviderOperationConsent> {
    [
        ProviderOperationConsentCategory::ImageSend,
        ProviderOperationConsentCategory::AiIndex,
        ProviderOperationConsentCategory::CostRisk,
    ]
    .into_iter()
    .map(|category| ProviderOperationConsent {
        profile_id,
        category,
        decision: ProviderOperationConsentDecision::SkipPrompt,
        updated_at: Utc.timestamp_opt(1_786_089_600, 0).single().unwrap(),
    })
    .collect()
}

#[derive(Clone)]
struct SeededBook {
    book_id: Uuid,
    section_id: Uuid,
    format: BookFormat,
    outline_sentinel: String,
    search_sentinel: String,
    whole_book_sentinel: String,
    stored_path: String,
}

async fn seed_profile(pool: &SqlitePool, profile_id: Uuid) {
    sqlx::query(
        "INSERT INTO provider_profiles (id, provider_kind, display_name, model_id, context_window_tokens, is_active, created_at, updated_at, validated_at) VALUES (?, 'openai', 'Book preparation profile', 'gpt-5.6', 128000, 1, ?, ?, ?)",
    )
    .bind(profile_id.to_string())
    .bind(FIXTURE_TIME)
    .bind(FIXTURE_TIME)
    .bind(FIXTURE_TIME)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "UPDATE app_settings SET active_provider_profile_id = ?, default_learning_profile_id = ?, context_mode = 'standard' WHERE id = 1",
    )
    .bind(profile_id.to_string())
    .bind(profile_id.to_string())
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "UPDATE teaching_preferences SET instruction = ?, revision = 1, updated_at = ? WHERE id = 1",
    )
    .bind(TEACHING_SENTINEL)
    .bind(FIXTURE_TIME)
    .execute(pool)
    .await
    .unwrap();
}

async fn seed_book(pool: &SqlitePool, format: BookFormat, name: &str) -> SeededBook {
    let book_id = Uuid::new_v4();
    let section_id = stable_section_id(book_id, 0);
    let block_id = Uuid::new_v4();
    let format_name = match format {
        BookFormat::Pdf => "pdf",
        BookFormat::Epub => "epub",
        BookFormat::Docx => "docx",
    };
    let locator = locator_for(&format, section_id, block_id, 0);
    let locator_json = serde_json::to_string(&locator).unwrap();
    let outline_sentinel = format!("OUTLINE_{name}_SENTINEL");
    let search_sentinel = format!("Orbital flux {name} SEARCH_SENTINEL");
    let whole_book_sentinel = format!("WHOLE_BOOK_{name}_MUST_NOT_LEAVE");
    let stored_path = format!("books/{book_id}/PRIVATE_PATH_{name}.{format_name}");
    let source_hash = sha256_hex(book_id.as_bytes());
    sqlx::query(
        "INSERT INTO books (id, sha256, title, format, original_filename, stored_path, import_status, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, 'ready', ?, ?)",
    )
    .bind(book_id.to_string())
    .bind(source_hash)
    .bind(format!("Synthetic {name}"))
    .bind(format_name)
    .bind(format!("synthetic.{format_name}"))
    .bind(&stored_path)
    .bind(FIXTURE_TIME)
    .bind(FIXTURE_TIME)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO sections (id, book_id, ordinal, title, locator_json) VALUES (?, ?, 0, ?, ?)",
    )
    .bind(section_id.to_string())
    .bind(book_id.to_string())
    .bind(&outline_sentinel)
    .bind(&locator_json)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO blocks (id, book_id, section_id, ordinal, kind, plain_text, locator_json) VALUES (?, ?, ?, 0, 'paragraph', ?, ?)",
    )
    .bind(block_id.to_string())
    .bind(book_id.to_string())
    .bind(section_id.to_string())
    .bind(&whole_book_sentinel)
    .bind(&locator_json)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO search_chunks (id, book_id, section_id, ordinal, text, locator_json, token_estimate) VALUES (?, ?, ?, 0, ?, ?, 12)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(book_id.to_string())
    .bind(section_id.to_string())
    .bind(&search_sentinel)
    .bind(&locator_json)
    .execute(pool)
    .await
    .unwrap();
    SeededBook {
        book_id,
        section_id,
        format,
        outline_sentinel,
        search_sentinel,
        whole_book_sentinel,
        stored_path,
    }
}

async fn add_outline_sections(pool: &SqlitePool, book: &SeededBook, total: u32) {
    for ordinal in 1..total {
        let section_id = stable_section_id(book.book_id, ordinal);
        let locator = locator_for(&book.format, section_id, Uuid::new_v4(), ordinal);
        sqlx::query(
            "INSERT INTO sections (id, book_id, ordinal, title, locator_json) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(section_id.to_string())
        .bind(book.book_id.to_string())
        .bind(i64::from(ordinal))
        .bind(format!("BOUNDED_OUTLINE_{ordinal:03}"))
        .bind(serde_json::to_string(&locator).unwrap())
        .execute(pool)
        .await
        .unwrap();
    }
}

fn locator_for(
    format: &BookFormat,
    section_id: Uuid,
    block_id: Uuid,
    ordinal: u32,
) -> DocumentLocator {
    match format {
        BookFormat::Pdf => {
            DocumentLocator::pdf(ordinal.saturating_add(1), ordinal.saturating_add(1), None)
                .unwrap()
        }
        BookFormat::Epub => DocumentLocator::epub(
            format!("epubcfi(/6/{})", ordinal.saturating_add(2)),
            section_id,
        )
        .unwrap(),
        BookFormat::Docx => DocumentLocator::docx(block_id, 0, block_id, 8).unwrap(),
    }
}

async fn seed_book_conversation(
    pool: &SqlitePool,
    book_id: Uuid,
    profile_id: Uuid,
    pairs: &[(&str, &str)],
) -> Uuid {
    let conversation_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO conversations (id, book_id, section_id, scope, anchor_kind, anchor_json, selected_text, created_at, updated_at) VALUES (?, ?, NULL, 'book', NULL, NULL, NULL, ?, ?)",
    )
    .bind(conversation_id.to_string())
    .bind(book_id.to_string())
    .bind(FIXTURE_TIME)
    .bind(FIXTURE_TIME)
    .execute(pool)
    .await
    .unwrap();
    for (pair_index, (user, assistant)) in pairs.iter().enumerate() {
        let user_ordinal = i64::try_from(pair_index * 2).unwrap();
        let action = if pair_index == 0 { "ask" } else { "continue" };
        sqlx::query(
            "INSERT INTO messages (id, conversation_id, ordinal, role, action, content, provider_id, model_id, citations_json, created_at) VALUES (?, ?, ?, 'user', ?, ?, NULL, NULL, NULL, ?)",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(conversation_id.to_string())
        .bind(user_ordinal)
        .bind(action)
        .bind(*user)
        .bind(FIXTURE_TIME)
        .execute(pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO messages (id, conversation_id, ordinal, role, action, content, provider_id, model_id, citations_json, created_at) VALUES (?, ?, ?, 'assistant', ?, ?, ?, 'gpt-5.6', '[]', ?)",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(conversation_id.to_string())
        .bind(user_ordinal + 1)
        .bind(action)
        .bind(*assistant)
        .bind(profile_id.to_string())
        .bind(FIXTURE_TIME)
        .execute(pool)
        .await
        .unwrap();
    }
    conversation_id
}

async fn seed_selection_conversation(
    pool: &SqlitePool,
    book: &SeededBook,
    profile_id: Uuid,
) -> Uuid {
    let conversation_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO conversations (id, book_id, section_id, scope, anchor_kind, anchor_json, selected_text, created_at, updated_at) VALUES (?, ?, ?, 'selection', 'text', '{}', 'selection sentinel', ?, ?)",
    )
    .bind(conversation_id.to_string())
    .bind(book.book_id.to_string())
    .bind(book.section_id.to_string())
    .bind(FIXTURE_TIME)
    .bind(FIXTURE_TIME)
    .execute(pool)
    .await
    .unwrap();
    for (ordinal, role, content) in [
        (0_i64, "user", "Selection question"),
        (1_i64, "assistant", "Selection answer"),
    ] {
        sqlx::query(
            "INSERT INTO messages (id, conversation_id, ordinal, role, action, content, provider_id, model_id, citations_json, created_at) VALUES (?, ?, ?, ?, 'ask', ?, ?, ?, ?, ?)",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(conversation_id.to_string())
        .bind(ordinal)
        .bind(role)
        .bind(content)
        .bind((role == "assistant").then(|| profile_id.to_string()))
        .bind((role == "assistant").then_some("gpt-5.6"))
        .bind((role == "assistant").then_some("[]"))
        .bind(FIXTURE_TIME)
        .execute(pool)
        .await
        .unwrap();
    }
    conversation_id
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
        counts.push(sqlx::query_scalar(query).fetch_one(pool).await.unwrap());
    }
    counts
}

async fn commit_indexed_page(
    pool: &SqlitePool,
    book_id: Uuid,
    profile_id: Uuid,
    text: &str,
    description: &str,
) -> (Uuid, Uuid) {
    let run_id = create_index_run(
        pool,
        CreateIndexRun {
            book_id,
            provider_profile_id: profile_id,
            analysis_schema_version: "textbooklens.page-analysis.v1".to_owned(),
            render_version: "book-preparation-render-v1".to_owned(),
            parser_version: "book-preparation-parser-v1".to_owned(),
        },
    )
    .await
    .unwrap();
    let page = state::queue(pool, run_id, 1, IndexQualityReason::NoText, None)
        .await
        .unwrap();
    let attempt_id = page.attempt_id.unwrap();
    state::claim_render(pool, page.page_id, attempt_id)
        .await
        .unwrap();
    state::mark_rendered(pool, page.page_id, attempt_id, &"c".repeat(64))
        .await
        .unwrap();
    state::claim_send(pool, page.page_id, attempt_id)
        .await
        .unwrap();
    state::mark_received(pool, page.page_id, attempt_id, &"d".repeat(64))
        .await
        .unwrap();
    commit_validated_page(
        pool,
        PageCommitRequest {
            page_id: page.page_id,
            attempt_id,
            page: &ValidatedPage {
                page_number: 1,
                review_reason: None,
                blocks: vec![ValidatedBlock {
                    ordinal: 0,
                    kind: IndexPageBlockKind::Paragraph,
                    plain_text: Some(text.to_owned()),
                    latex: None,
                    table_cells: None,
                    visual_description: Some(description.to_owned()),
                    bounds: Some(NormalizedRect::new(0.1, 0.1, 0.8, 0.2).unwrap()),
                    source: DurableContentSource::AiTranscribed,
                }],
            },
        },
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    let block_id: String =
        sqlx::query_scalar("SELECT id FROM index_page_blocks WHERE page_id = ? AND ordinal = 0")
            .bind(page.page_id.to_string())
            .fetch_one(pool)
            .await
            .unwrap();
    (page.page_id, Uuid::parse_str(&block_id).unwrap())
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|value| format!("{value:02x}"))
        .collect()
}
