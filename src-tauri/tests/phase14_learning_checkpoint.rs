use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use async_trait::async_trait;
use futures_util::stream;
use secrecy::SecretString;
use sqlx::{Row, SqlitePool};
use textbooklens_lib::{
    ai::{error::AiError, provider::ProviderStream, registry::ProviderCapabilityRegistry},
    credentials::CredentialStore,
    db::{
        Database,
        annotations::list_annotation_markers,
        conversations::{
            DeleteBookConversation, LearningRepository, NewBookQuestionCompletion,
            NewSelectionCompletion, list_book_conversation_summaries, load_book_conversation,
        },
        corrections::{SaveIndexCorrection, correction_value_sha256, save_index_correction},
        indexing::{CreateIndexRun, create_index_run},
        messages::CompletedAssistantMessage,
        notes::{CreateNote, create_note},
        overview::{OVERVIEW_READ_QUERY_COUNT, get_learning_overview},
        providers,
    },
    domain::{
        BookFormat, Citation, CitationReviewStatus, ContentAnchor, ContentSource, DocumentLocator,
        IndexCorrectionValueKind, IndexPageBlockKind, IndexQualityReason, IndexReviewReason,
        LearningAction, LearningOverviewSource, LearningRequestStatus, NormalizedRect,
        PrepareBookLearningRequestMetadata, SelectionAnchor, TextQuote, UnifiedStreamEvent,
        stable_block_id, stable_index_page_block_id, stable_section_id,
    },
    errors::{AppError, AppErrorCode, AppResult},
    indexing::{
        commit::{PageCommitRequest, commit_validated_page},
        state,
        validator::{ValidatedBlock, ValidatedPage},
    },
    learning::{
        book_preparation::{BookLearningPreparationService, BookPreparationRegistry},
        orchestrator::LearningOrchestrator,
        overview::LearningOverviewService,
        registry::{
            BookFollowupRequestContext, LearningPersistenceTarget, LearningRequestContext,
            LearningRequestRegistry, NewBookQuestionRequestContext,
        },
    },
    maintenance::gate::MaintenanceGate,
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const TIME: &str = "2026-08-06T00:00:00.000Z";
const ANALYSIS_SCHEMA: &str = "textbooklens.page-analysis.v1";
const LOCAL_TEXT: &str = "P14_LOCAL_TEXT_PRIVATE_SENTINEL orbital flux";
const TRANSCRIPTION: &str = "P14_AI_TRANSCRIPTION_PRIVATE_SENTINEL orbital flux evidence";
const DESCRIPTION: &str = "P14_AI_DESCRIPTION_PRIVATE_SENTINEL";
const CORRECTION: &str = "P14_USER_CORRECTION_PRIVATE_SENTINEL corrected flux";
const NOTE: &str = "P14_USER_NOTE_PRIVATE_SENTINEL";
const TEACHING: &str = "P14_TEACHING_PRIVATE_SENTINEL use one bounded counterexample";
const QUESTION: &str = "How does orbital flux connect to the reviewed evidence?";
const ANSWER: &str = "P14_ALLOWED_ANSWER_SENTINEL **bounded answer** [TL-C1]";
const STREAM_ANSWER: &str = "P14_ALLOWED_STREAM_ANSWER **bounded answer**";
const HIDDEN: &str = "P14_HIDDEN_REASONING_PRIVATE_SENTINEL";
const PATH_SENTINEL: &str = "P14_PRIVATE_PATH_SENTINEL";
const CREDENTIAL_SENTINEL: &str = "P14_PRIVATE_CREDENTIAL_SENTINEL";

#[derive(Clone)]
struct SeededBook {
    id: Uuid,
    section_id: Uuid,
    locator: DocumentLocator,
    outline: String,
    retrieval: String,
    whole_book: String,
    stored_path: String,
}

#[derive(Default)]
struct CountingCredentialStore {
    values: Mutex<HashMap<String, SecretString>>,
    reads: AtomicUsize,
}

impl CountingCredentialStore {
    fn reads(&self) -> usize {
        self.reads.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl CredentialStore for CountingCredentialStore {
    async fn set(&self, key: &str, value: SecretString) -> AppResult<()> {
        self.values.lock().unwrap().insert(key.to_owned(), value);
        Ok(())
    }

    async fn get(&self, key: &str) -> AppResult<SecretString> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        self.values
            .lock()
            .unwrap()
            .get(key)
            .cloned()
            .ok_or_else(|| AppError::credential_store("synthetic credential missing"))
    }

    async fn delete(&self, key: &str) -> AppResult<()> {
        self.values.lock().unwrap().remove(key);
        Ok(())
    }
}

#[test]
fn synthetic_three_format_overview_is_local_deterministic_and_restart_stable() {
    let temporary = tempfile::tempdir().unwrap();
    let database_path = temporary.path().join("phase14-overview.sqlite3");
    let database = Database::open(&database_path).unwrap();
    let credential_store = Arc::new(CountingCredentialStore::default());

    let (target_id, expected) = tauri::async_runtime::block_on(async {
        let profile_id = seed_profile(database.pool()).await;
        credential_store
            .set(
                &providers::credential_key(profile_id),
                SecretString::from(CREDENTIAL_SENTINEL),
            )
            .await
            .unwrap();
        let pdf = seed_book(database.pool(), BookFormat::Pdf, "pdf").await;
        let epub = seed_book(database.pool(), BookFormat::Epub, "epub").await;
        let docx = seed_book(database.pool(), BookFormat::Docx, "docx").await;
        let decoy = seed_book(database.pool(), BookFormat::Pdf, "decoy").await;
        set_teaching(database.pool(), TEACHING, 1).await;

        let committed = commit_reviewed_page(database.pool(), pdf.id, profile_id).await;
        let correction = save_index_correction(
            database.pool(),
            SaveIndexCorrection {
                book_id: pdf.id,
                page_id: committed.page_id,
                target_block_id: committed.correctable_block_id,
                target_content_version: 1,
                value_kind: IndexCorrectionValueKind::Text,
                original_value_sha256: correction_value_sha256("provider text to correct"),
                corrected_value: CORRECTION.to_owned(),
                expected_revision: 0,
            },
        )
        .await
        .unwrap();
        assert_eq!(correction.revision, 1);
        create_note(
            database.pool(),
            CreateNote {
                book_id: pdf.id,
                section_id: pdf.section_id,
                anchor: text_anchor(&pdf, LOCAL_TEXT),
                selected_text: Some(LOCAL_TEXT.to_owned()),
                note_text: NOTE.to_owned(),
            },
        )
        .await
        .unwrap();
        let citation = local_citation(&pdf);
        let persisted = LearningRepository::new(database.pool().clone())
            .persist_new_book_question(NewBookQuestionCompletion {
                book_id: pdf.id,
                question: QUESTION.to_owned(),
                assistant: CompletedAssistantMessage {
                    provider_profile_id: profile_id,
                    model_id: "gpt-5.6".to_owned(),
                    answer: ANSWER.to_owned(),
                    available_citations: vec![citation.clone()],
                },
            })
            .await
            .unwrap();
        assert_eq!(persisted.annotation_id, None);
        let history = load_book_conversation(database.pool(), pdf.id, persisted.conversation_id)
            .await
            .unwrap();
        assert_eq!(history.messages[1].citations, vec![citation]);
        assert_eq!(
            history.messages[1].citations[0].review_status,
            CitationReviewStatus::NotRequired
        );

        for book in [&epub, &docx] {
            let overview = get_learning_overview(database.pool(), book.id)
                .await
                .unwrap();
            assert_eq!(overview.section_count, 1);
            assert_eq!(
                overview.sources[LearningOverviewSource::LocalText.index()].item_count,
                1
            );
        }
        let fts_transcribed: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM index_search_chunks_fts WHERE index_search_chunks_fts MATCH 'TRANSCRIPTION'",
        )
        .fetch_one(database.pool())
        .await
        .unwrap();
        let fts_corrected: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM index_search_chunks_fts WHERE index_search_chunks_fts MATCH 'CORRECTION'",
        )
        .fetch_one(database.pool())
        .await
        .unwrap();
        assert_eq!((fts_transcribed, fts_corrected), (1, 1));

        let service =
            LearningOverviewService::new(database.pool().clone(), MaintenanceGate::default());
        let before_rows = protected_row_count(database.pool()).await;
        let mut witness = database.pool().acquire().await.unwrap();
        let before_version: i64 = sqlx::query_scalar("PRAGMA data_version")
            .fetch_one(&mut *witness)
            .await
            .unwrap();
        let first = service.get(pdf.id).await.unwrap();
        let second = service.get(pdf.id).await.unwrap();
        let after_version: i64 = sqlx::query_scalar("PRAGMA data_version")
            .fetch_one(&mut *witness)
            .await
            .unwrap();
        assert_eq!(first, second);
        assert_eq!(OVERVIEW_READ_QUERY_COUNT, 11);
        assert_eq!(before_version, after_version);
        assert_eq!(before_rows, protected_row_count(database.pool()).await);
        assert_eq!(credential_store.reads(), 0);
        assert_eq!(
            source_tuple(&first, LearningOverviewSource::LocalText),
            (1, 1, 0, true)
        );
        assert_eq!(
            source_tuple(&first, LearningOverviewSource::AiTranscribed),
            (1, 0, 1, true)
        );
        assert_eq!(
            source_tuple(&first, LearningOverviewSource::AiDescription),
            (1, 0, 1, false)
        );
        assert_eq!(
            source_tuple(&first, LearningOverviewSource::UserCorrected),
            (1, 0, 1, true)
        );
        assert_eq!(
            source_tuple(&first, LearningOverviewSource::UserNote),
            (1, 1, 0, false)
        );
        assert_eq!(
            source_tuple(&first, LearningOverviewSource::HistorySummary),
            (1, 0, 0, false)
        );
        assert_eq!(first.activity.completed_conversation_count, 1);
        assert_eq!(first.activity.completed_exchange_count, 1);
        assert_eq!(first.activity.citation_count, 1);
        let dto = serde_json::to_string(&first).unwrap();
        for forbidden in privacy_sentinels(&pdf, &decoy) {
            assert!(!dto.contains(&forbidden), "overview DTO leaked {forbidden}");
        }
        (pdf.id, first)
    });
    tauri::async_runtime::block_on(database.pool().close());
    drop(database);

    let reopened = Database::open(&database_path).unwrap();
    let actual =
        tauri::async_runtime::block_on(get_learning_overview(reopened.pool(), target_id)).unwrap();
    assert_eq!(actual, expected);
    assert_eq!(credential_store.reads(), 0);
}

#[test]
fn synthetic_book_prepare_uses_only_bounded_authoritative_same_book_context() {
    let temporary = tempfile::tempdir().unwrap();
    let database = Database::open(temporary.path().join("phase14-prepare.sqlite3")).unwrap();
    let credentials = Arc::new(CountingCredentialStore::default());

    tauri::async_runtime::block_on(async {
        let profile_id = seed_profile(database.pool()).await;
        credentials
            .set(
                &providers::credential_key(profile_id),
                SecretString::from(CREDENTIAL_SENTINEL),
            )
            .await
            .unwrap();
        let target = seed_book(database.pool(), BookFormat::Pdf, "prepare-target").await;
        let decoy = seed_book(database.pool(), BookFormat::Epub, "prepare-decoy").await;
        set_teaching(database.pool(), TEACHING, 1).await;
        let target_history = persist_book_pair(
            database.pool(),
            target.id,
            profile_id,
            "P14_TARGET_HISTORY_QUESTION",
            "P14_TARGET_HISTORY_ANSWER",
        )
        .await;
        let _same_book_decoy = persist_book_pair(
            database.pool(),
            target.id,
            profile_id,
            "P14_SAME_BOOK_OTHER_CONVERSATION",
            "P14_SAME_BOOK_OTHER_ANSWER",
        )
        .await;
        let _other_book = persist_book_pair(
            database.pool(),
            decoy.id,
            profile_id,
            "P14_OTHER_BOOK_HISTORY",
            "P14_OTHER_BOOK_ANSWER",
        )
        .await;
        let selection = LearningRepository::new(database.pool().clone())
            .persist_new_selection(NewSelectionCompletion {
                book_id: target.id,
                section_id: target.section_id,
                anchor: text_anchor(&target, "P14_SELECTION_PRIVATE_SENTINEL"),
                selected_text: Some("P14_SELECTION_PRIVATE_SENTINEL".to_owned()),
                action: LearningAction::Ask,
                question: "P14_SELECTION_QUESTION".to_owned(),
                assistant: CompletedAssistantMessage {
                    provider_profile_id: profile_id,
                    model_id: "gpt-5.6".to_owned(),
                    answer: "P14_SELECTION_ANSWER".to_owned(),
                    available_citations: Vec::new(),
                },
            })
            .await
            .unwrap();
        assert_ne!(selection.annotation_id, Uuid::nil());

        let service = BookLearningPreparationService::new(
            database.pool().clone(),
            BookPreparationRegistry::default(),
            ProviderCapabilityRegistry::load_embedded().unwrap(),
            credentials.clone(),
            MaintenanceGate::default(),
        );
        let invalid_reads = credentials.reads();
        let oversized = service
            .prepare(PrepareBookLearningRequestMetadata::New {
                book_id: target.id,
                question: "x".repeat(16_385),
            })
            .await
            .unwrap_err();
        assert_eq!(oversized.code, AppErrorCode::InvalidInput);
        assert_eq!(credentials.reads(), invalid_reads);

        sqlx::query(
            "UPDATE provider_profiles SET model_id = 'unsupported-checkpoint-model', updated_at = '2026-08-06T00:00:01.000Z' WHERE id = ?",
        )
        .bind(profile_id.to_string())
        .execute(database.pool())
        .await
        .unwrap();
        let unsupported_reads = credentials.reads();
        let unsupported = service
            .prepare(PrepareBookLearningRequestMetadata::New {
                book_id: target.id,
                question: QUESTION.to_owned(),
            })
            .await
            .unwrap_err();
        assert_eq!(unsupported.code, AppErrorCode::InvalidInput);
        assert_eq!(unsupported.stable_code(), "UNSUPPORTED_PROVIDER_CAPABILITY");
        assert_eq!(credentials.reads(), unsupported_reads);
        sqlx::query(
            "UPDATE provider_profiles SET model_id = 'gpt-5.6', updated_at = ? WHERE id = ?",
        )
        .bind(TIME)
        .bind(profile_id.to_string())
        .execute(database.pool())
        .await
        .unwrap();

        let race = service
            .prepare(PrepareBookLearningRequestMetadata::New {
                book_id: target.id,
                question: QUESTION.to_owned(),
            })
            .await
            .unwrap();
        let reads_before_race = credentials.reads();
        set_teaching(database.pool(), "P14_BINDING_RACE_SENTINEL", 2).await;
        let race_error = service
            .consume_for_execution(race.preparation_id)
            .await
            .unwrap_err();
        assert_eq!(race_error.code, AppErrorCode::RequestConflict);
        assert_eq!(credentials.reads(), reads_before_race);
        set_teaching(database.pool(), TEACHING, 3).await;

        let summary = service
            .prepare(PrepareBookLearningRequestMetadata::Continue {
                book_id: target.id,
                conversation_id: target_history,
                question: QUESTION.to_owned(),
            })
            .await
            .unwrap();
        assert_eq!(summary.citation_count, 1);
        let prepared = service
            .consume_for_execution(summary.preparation_id)
            .await
            .unwrap();
        assert_eq!(prepared.book_id(), target.id);
        assert_eq!(prepared.conversation_id(), Some(target_history));
        assert_eq!(prepared.expected_next_ordinal(), Some(2));
        assert_eq!(prepared.current_question().unwrap(), QUESTION);
        assert_eq!(prepared.teaching_instruction().instruction, TEACHING);
        let chat_request = prepared.chat_request();
        assert!(chat_request.system.contains("\"source\":\"directory\""));
        assert!(chat_request.system.contains("\"source\":\"local_text\""));
        let request = serde_json::to_string(&chat_request).unwrap();
        for required in [
            TEACHING,
            target.outline.as_str(),
            target.retrieval.as_str(),
            "P14_TARGET_HISTORY_QUESTION",
            "P14_TARGET_HISTORY_ANSWER",
            QUESTION,
        ] {
            assert!(
                request.contains(required),
                "missing prepared context {required}"
            );
        }
        for forbidden in [
            target.whole_book.as_str(),
            decoy.outline.as_str(),
            decoy.retrieval.as_str(),
            decoy.whole_book.as_str(),
            target.stored_path.as_str(),
            decoy.stored_path.as_str(),
            "P14_SAME_BOOK_OTHER_CONVERSATION",
            "P14_SAME_BOOK_OTHER_ANSWER",
            "P14_OTHER_BOOK_HISTORY",
            "P14_OTHER_BOOK_ANSWER",
            "P14_SELECTION_PRIVATE_SENTINEL",
            "P14_SELECTION_QUESTION",
            "P14_SELECTION_ANSWER",
            CREDENTIAL_SENTINEL,
            DESCRIPTION,
            HIDDEN,
        ] {
            assert!(
                !request.contains(forbidden),
                "forbidden prepared context {forbidden}"
            );
        }
        for internal_id in [target.id, target.section_id, profile_id, target_history] {
            assert!(!request.contains(&internal_id.to_string()));
        }
        assert!(prepared.packed_context().estimated_input_tokens <= 32_768);
        assert_eq!(prepared.packed_context().citations.len(), 1);
        assert_eq!(prepared.packed_context().citations[0].book_id, target.id);

        drop(prepared);
        add_outline_sections(database.pool(), &target, 300).await;
        let bounded = service
            .prepare(PrepareBookLearningRequestMetadata::New {
                book_id: target.id,
                question: QUESTION.to_owned(),
            })
            .await
            .unwrap();
        assert!(
            bounded.omitted_source_count > 0,
            "300-item TOC must be bounded"
        );
        drop(
            service
                .consume_for_execution(bounded.preparation_id)
                .await
                .unwrap(),
        );
    });
}

#[test]
fn synthetic_provider_stream_is_atomic_restartable_book_history_without_markers() {
    let temporary = tempfile::tempdir().unwrap();
    let database_path = temporary.path().join("phase14-stream.sqlite3");
    let database = Database::open(&database_path).unwrap();
    let book_id = Uuid::new_v4();
    let profile_id = Uuid::new_v4();

    let conversation_id = tauri::async_runtime::block_on(async {
        seed_minimal_book(database.pool(), book_id, BookFormat::Docx).await;
        seed_profile_with_id(database.pool(), profile_id).await;
        let registry = LearningRequestRegistry::default();
        let orchestrator = LearningOrchestrator::new(registry.clone(), database.pool().clone());

        let eof = registry
            .create(new_book_context(
                book_id,
                profile_id,
                "captured-model-v1",
                "EOF question",
            ))
            .unwrap();
        orchestrator
            .run_stream(
                eof.request_id,
                provider_stream(vec![UnifiedStreamEvent::TextDelta {
                    text: "P14_PARTIAL_MEMORY_ONLY_SENTINEL".to_owned(),
                }]),
            )
            .await;
        assert_eq!(
            registry.snapshot(eof.request_id).unwrap().status,
            LearningRequestStatus::Failed
        );

        let stopped = registry
            .create(new_book_context(
                book_id,
                profile_id,
                "captured-model-v1",
                "Stopped question",
            ))
            .unwrap();
        registry.cancel(stopped.request_id).unwrap();
        orchestrator
            .run_stream(
                stopped.request_id,
                provider_stream(vec![
                    UnifiedStreamEvent::TextDelta {
                        text: "late".to_owned(),
                    },
                    UnifiedStreamEvent::Completed,
                ]),
            )
            .await;
        assert_eq!(
            registry.snapshot(stopped.request_id).unwrap().status,
            LearningRequestStatus::Cancelled
        );

        let completed = registry
            .create(new_book_context(
                book_id,
                profile_id,
                "captured-model-v1",
                QUESTION,
            ))
            .unwrap();
        orchestrator
            .run_stream(
                completed.request_id,
                provider_stream(vec![
                    UnifiedStreamEvent::TextDelta {
                        text: STREAM_ANSWER.to_owned(),
                    },
                    UnifiedStreamEvent::Completed,
                    UnifiedStreamEvent::TextDelta {
                        text: "P14_LATE_DELTA_SENTINEL".to_owned(),
                    },
                    UnifiedStreamEvent::Completed,
                ]),
            )
            .await;
        let snapshot = registry.snapshot(completed.request_id).unwrap();
        assert_eq!(snapshot.status, LearningRequestStatus::Completed);
        registry.cancel(completed.request_id).unwrap();
        assert_eq!(
            registry.snapshot(completed.request_id).unwrap().status,
            LearningRequestStatus::Completed
        );
        let conversation_id = snapshot.conversation_id.unwrap();
        let history = load_book_conversation(database.pool(), book_id, conversation_id)
            .await
            .unwrap();
        assert_eq!(
            history.scope,
            textbooklens_lib::domain::ConversationScope::Book
        );
        assert_eq!(history.messages.len(), 2);
        assert_eq!(history.messages[1].provider_id, Some(profile_id));
        assert_eq!(
            history.messages[1].model_id.as_deref(),
            Some("captured-model-v1")
        );
        assert!(
            list_annotation_markers(database.pool(), book_id)
                .await
                .unwrap()
                .is_empty()
        );
        assert_database_counts(database.pool(), (1, 2, 0)).await;
        database.pool().close().await;
        conversation_id
    });
    drop(database);

    let reopened = Database::open(&database_path).unwrap();
    tauri::async_runtime::block_on(async {
        let fresh_registry = LearningRequestRegistry::default();
        assert_eq!(
            fresh_registry.snapshot(conversation_id).unwrap_err().code,
            AppErrorCode::NotFound
        );
        let durable = load_book_conversation(reopened.pool(), book_id, conversation_id)
            .await
            .unwrap();
        assert_eq!(durable.messages.len(), 2);
        let summaries = list_book_conversation_summaries(reopened.pool(), book_id)
            .await
            .unwrap();
        assert_eq!(summaries.len(), 1);
        assert!(summaries[0].first_question_preview.chars().count() <= 280);

        let registry = LearningRequestRegistry::default();
        let orchestrator = LearningOrchestrator::new(registry.clone(), reopened.pool().clone());
        let followup = registry
            .create(Arc::new(LearningRequestContext {
                target: LearningPersistenceTarget::BookFollowup(BookFollowupRequestContext {
                    book_id,
                    conversation_id,
                    expected_next_ordinal: 2,
                    question: "Current captured follow-up".to_owned(),
                }),
                provider_profile_id: profile_id,
                model_id: "captured-current-model-v2".to_owned(),
                available_citations: Vec::new(),
            }))
            .unwrap();
        assert_eq!(
            registry
                .create(Arc::new(LearningRequestContext {
                    target: LearningPersistenceTarget::BookFollowup(BookFollowupRequestContext {
                        book_id,
                        conversation_id,
                        expected_next_ordinal: 2,
                        question: "Conflicting follow-up".to_owned(),
                    }),
                    provider_profile_id: profile_id,
                    model_id: "captured-current-model-v2".to_owned(),
                    available_citations: Vec::new(),
                }))
                .unwrap_err()
                .code,
            AppErrorCode::RequestConflict
        );
        orchestrator
            .run_stream(
                followup.request_id,
                provider_stream(vec![
                    UnifiedStreamEvent::TextDelta {
                        text: "Current follow-up answer".to_owned(),
                    },
                    UnifiedStreamEvent::Completed,
                ]),
            )
            .await;
        let history = load_book_conversation(reopened.pool(), book_id, conversation_id)
            .await
            .unwrap();
        assert_eq!(history.messages.len(), 4);
        assert_eq!(
            history.messages[1].model_id.as_deref(),
            Some("captured-model-v1")
        );
        assert_eq!(
            history.messages[3].model_id.as_deref(),
            Some("captured-current-model-v2")
        );

        let deletion = registry
            .begin_conversation_deletion(conversation_id)
            .unwrap();
        assert_eq!(
            registry
                .create(Arc::new(LearningRequestContext {
                    target: LearningPersistenceTarget::BookFollowup(BookFollowupRequestContext {
                        book_id,
                        conversation_id,
                        expected_next_ordinal: 4,
                        question: "Late resurrection attempt".to_owned(),
                    }),
                    provider_profile_id: profile_id,
                    model_id: "late-model".to_owned(),
                    available_citations: Vec::new(),
                }))
                .unwrap_err()
                .code,
            AppErrorCode::RequestConflict
        );
        LearningRepository::new(reopened.pool().clone())
            .delete_book_conversation(DeleteBookConversation {
                book_id,
                conversation_id,
            })
            .await
            .unwrap();
        drop(deletion);
        assert_database_counts(reopened.pool(), (0, 0, 0)).await;
        assert!(
            list_annotation_markers(reopened.pool(), book_id)
                .await
                .unwrap()
                .is_empty()
        );
    });

    let bytes = std::fs::read(&database_path).unwrap();
    for forbidden in [
        "P14_PARTIAL_MEMORY_ONLY_SENTINEL",
        "P14_LATE_DELTA_SENTINEL",
        HIDDEN,
        CREDENTIAL_SENTINEL,
    ] {
        assert!(
            !contains_bytes(&bytes, forbidden.as_bytes()),
            "database leaked {forbidden}"
        );
    }
}

struct CommittedPage {
    page_id: Uuid,
    correctable_block_id: Uuid,
}

async fn seed_profile(pool: &SqlitePool) -> Uuid {
    let profile_id = Uuid::new_v4();
    seed_profile_with_id(pool, profile_id).await;
    profile_id
}

async fn seed_profile_with_id(pool: &SqlitePool, profile_id: Uuid) {
    sqlx::query(
        "INSERT INTO provider_profiles (id, provider_kind, display_name, model_id, context_window_tokens, is_active, created_at, updated_at, validated_at) VALUES (?, 'openai', 'Synthetic profile', 'gpt-5.6', 128000, 1, ?, ?, ?)",
    )
    .bind(profile_id.to_string())
    .bind(TIME)
    .bind(TIME)
    .bind(TIME)
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
    for category in ["image_send", "ai_index", "cost_risk"] {
        sqlx::query(
            "INSERT INTO provider_operation_consents (profile_id, category, decision, updated_at) VALUES (?, ?, 'skip_prompt', ?)",
        )
        .bind(profile_id.to_string())
        .bind(category)
        .bind(TIME)
        .execute(pool)
        .await
        .unwrap();
    }
}

async fn set_teaching(pool: &SqlitePool, instruction: &str, revision: i64) {
    sqlx::query(
        "UPDATE teaching_preferences SET instruction = ?, revision = ?, updated_at = ? WHERE id = 1",
    )
    .bind(instruction)
    .bind(revision)
    .bind(TIME)
    .execute(pool)
    .await
    .unwrap();
}

async fn seed_book(pool: &SqlitePool, format: BookFormat, name: &str) -> SeededBook {
    let id = Uuid::new_v4();
    let section_id = stable_section_id(id, 0);
    let block_id = stable_block_id(id, 0, 0);
    let format_name = format_name(&format);
    let locator = locator_for(&format, section_id, block_id, 0);
    let outline = format!("P14_OUTLINE_{name}_SENTINEL");
    let retrieval = format!("Orbital flux P14_RETRIEVAL_{name}_SENTINEL");
    let whole_book = format!("P14_WHOLE_BOOK_{name}_SENTINEL");
    let stored_path = format!("books/{id}/{PATH_SENTINEL}_{name}.{format_name}");
    sqlx::query(
        "INSERT INTO books (id, sha256, title, format, original_filename, stored_path, import_status, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, 'ready', ?, ?)",
    )
    .bind(id.to_string())
    .bind(hex_hash(id))
    .bind(format!("Synthetic {name}"))
    .bind(format_name)
    .bind(format!("synthetic.{format_name}"))
    .bind(&stored_path)
    .bind(TIME)
    .bind(TIME)
    .execute(pool)
    .await
    .unwrap();
    let locator_json = serde_json::to_string(&locator).unwrap();
    sqlx::query(
        "INSERT INTO sections (id, book_id, ordinal, title, locator_json) VALUES (?, ?, 0, ?, ?)",
    )
    .bind(section_id.to_string())
    .bind(id.to_string())
    .bind(&outline)
    .bind(&locator_json)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO blocks (id, book_id, section_id, ordinal, kind, plain_text, locator_json) VALUES (?, ?, ?, 0, 'paragraph', ?, ?)",
    )
    .bind(block_id.to_string())
    .bind(id.to_string())
    .bind(section_id.to_string())
    .bind(if name == "pdf" { LOCAL_TEXT } else { &whole_book })
    .bind(&locator_json)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO search_chunks (id, book_id, section_id, ordinal, text, locator_json, token_estimate) VALUES (?, ?, ?, 0, ?, ?, 12)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(id.to_string())
    .bind(section_id.to_string())
    .bind(&retrieval)
    .bind(&locator_json)
    .execute(pool)
    .await
    .unwrap();
    SeededBook {
        id,
        section_id,
        locator,
        outline,
        retrieval,
        whole_book,
        stored_path,
    }
}

async fn seed_minimal_book(pool: &SqlitePool, id: Uuid, format: BookFormat) {
    let section_id = stable_section_id(id, 0);
    let block_id = stable_block_id(id, 0, 0);
    let locator = locator_for(&format, section_id, block_id, 0);
    sqlx::query(
        "INSERT INTO books (id, sha256, title, format, original_filename, stored_path, import_status, created_at, updated_at) VALUES (?, ?, 'Stream book', ?, 'stream.synthetic', 'books/stream/original.synthetic', 'ready', ?, ?)",
    )
    .bind(id.to_string())
    .bind(hex_hash(id))
    .bind(format_name(&format))
    .bind(TIME)
    .bind(TIME)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO sections (id, book_id, ordinal, title, locator_json) VALUES (?, ?, 0, 'Stream section', ?)",
    )
    .bind(section_id.to_string())
    .bind(id.to_string())
    .bind(serde_json::to_string(&locator).unwrap())
    .execute(pool)
    .await
    .unwrap();
}

async fn add_outline_sections(pool: &SqlitePool, book: &SeededBook, total: u32) {
    for ordinal in 1..total {
        let section_id = stable_section_id(book.id, ordinal);
        let locator = locator_for(&BookFormat::Pdf, section_id, Uuid::new_v4(), ordinal);
        sqlx::query(
            "INSERT INTO sections (id, book_id, ordinal, title, locator_json) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(section_id.to_string())
        .bind(book.id.to_string())
        .bind(i64::from(ordinal))
        .bind(format!("P14_BOUNDED_OUTLINE_{ordinal:03}"))
        .bind(serde_json::to_string(&locator).unwrap())
        .execute(pool)
        .await
        .unwrap();
    }
}

async fn commit_reviewed_page(pool: &SqlitePool, book_id: Uuid, profile_id: Uuid) -> CommittedPage {
    let run_id = create_index_run(
        pool,
        CreateIndexRun {
            book_id,
            provider_profile_id: profile_id,
            analysis_schema_version: ANALYSIS_SCHEMA.to_owned(),
            render_version: "phase14-render-v1".to_owned(),
            parser_version: "phase14-parser-v1".to_owned(),
        },
    )
    .await
    .unwrap();
    let page = state::queue(pool, run_id, 1, IndexQualityReason::NoText, None)
        .await
        .unwrap();
    let attempt = page.attempt_id.unwrap();
    state::claim_render(pool, page.page_id, attempt)
        .await
        .unwrap();
    state::mark_rendered(pool, page.page_id, attempt, &"c".repeat(64))
        .await
        .unwrap();
    state::claim_send(pool, page.page_id, attempt)
        .await
        .unwrap();
    state::mark_received(pool, page.page_id, attempt, &"d".repeat(64))
        .await
        .unwrap();
    commit_validated_page(
        pool,
        PageCommitRequest {
            page_id: page.page_id,
            attempt_id: attempt,
            page: &ValidatedPage {
                page_number: 1,
                review_reason: Some(IndexReviewReason::TextContradiction),
                blocks: vec![
                    ValidatedBlock {
                        ordinal: 0,
                        kind: IndexPageBlockKind::Paragraph,
                        plain_text: Some(TRANSCRIPTION.to_owned()),
                        latex: None,
                        table_cells: None,
                        visual_description: Some(DESCRIPTION.to_owned()),
                        bounds: Some(NormalizedRect::new(0.1, 0.1, 0.8, 0.2).unwrap()),
                        source: ContentSource::AiTranscribed,
                    },
                    ValidatedBlock {
                        ordinal: 1,
                        kind: IndexPageBlockKind::Paragraph,
                        plain_text: Some("provider text to correct".to_owned()),
                        latex: None,
                        table_cells: None,
                        visual_description: None,
                        bounds: Some(NormalizedRect::new(0.1, 0.4, 0.8, 0.2).unwrap()),
                        source: ContentSource::AiTranscribed,
                    },
                ],
            },
        },
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    CommittedPage {
        page_id: page.page_id,
        correctable_block_id: stable_index_page_block_id(book_id, 1, run_id, ANALYSIS_SCHEMA, 1),
    }
}

async fn persist_book_pair(
    pool: &SqlitePool,
    book_id: Uuid,
    profile_id: Uuid,
    question: &str,
    answer: &str,
) -> Uuid {
    LearningRepository::new(pool.clone())
        .persist_new_book_question(NewBookQuestionCompletion {
            book_id,
            question: question.to_owned(),
            assistant: CompletedAssistantMessage {
                provider_profile_id: profile_id,
                model_id: "gpt-5.6".to_owned(),
                answer: answer.to_owned(),
                available_citations: Vec::new(),
            },
        })
        .await
        .unwrap()
        .conversation_id
}

fn local_citation(book: &SeededBook) -> Citation {
    Citation::new(
        "TL-C1".to_owned(),
        "Synthetic local evidence".to_owned(),
        book.id,
        Some(book.section_id),
        book.locator.clone(),
        ContentSource::LocalText,
        CitationReviewStatus::NotRequired,
    )
    .unwrap()
}

fn text_anchor(book: &SeededBook, exact: &str) -> ContentAnchor {
    ContentAnchor::Text {
        selection: SelectionAnchor {
            locator: book.locator.clone(),
            quote: TextQuote::new(exact.to_owned(), String::new(), String::new()).unwrap(),
            section_id: Some(book.section_id),
        },
    }
}

fn new_book_context(
    book_id: Uuid,
    profile_id: Uuid,
    model_id: &str,
    question: &str,
) -> Arc<LearningRequestContext> {
    Arc::new(LearningRequestContext {
        target: LearningPersistenceTarget::NewBookQuestion(NewBookQuestionRequestContext {
            book_id,
            question: question.to_owned(),
        }),
        provider_profile_id: profile_id,
        model_id: model_id.to_owned(),
        available_citations: Vec::new(),
    })
}

fn provider_stream(events: Vec<UnifiedStreamEvent>) -> ProviderStream {
    Box::pin(stream::iter(
        events.into_iter().map(Ok::<UnifiedStreamEvent, AiError>),
    ))
}

fn locator_for(
    format: &BookFormat,
    section_id: Uuid,
    block_id: Uuid,
    ordinal: u32,
) -> DocumentLocator {
    match format {
        BookFormat::Pdf => DocumentLocator::pdf(ordinal + 1, ordinal + 1, None).unwrap(),
        BookFormat::Epub => {
            DocumentLocator::epub(format!("epubcfi(/6/{})", ordinal + 2), section_id).unwrap()
        }
        BookFormat::Docx => DocumentLocator::docx(block_id, 0, block_id, 8).unwrap(),
    }
}

fn format_name(format: &BookFormat) -> &'static str {
    match format {
        BookFormat::Pdf => "pdf",
        BookFormat::Epub => "epub",
        BookFormat::Docx => "docx",
    }
}

fn hex_hash(id: Uuid) -> String {
    id.as_simple().to_string().repeat(2)
}

fn source_tuple(
    overview: &textbooklens_lib::domain::LearningOverview,
    source: LearningOverviewSource,
) -> (u32, u32, u32, bool) {
    let value = &overview.sources[source.index()];
    (
        value.item_count,
        value.covered_section_count,
        value.covered_page_count,
        value.quoteable_as_textbook,
    )
}

fn privacy_sentinels(target: &SeededBook, decoy: &SeededBook) -> Vec<String> {
    vec![
        LOCAL_TEXT.to_owned(),
        TRANSCRIPTION.to_owned(),
        DESCRIPTION.to_owned(),
        CORRECTION.to_owned(),
        NOTE.to_owned(),
        TEACHING.to_owned(),
        QUESTION.to_owned(),
        ANSWER.to_owned(),
        HIDDEN.to_owned(),
        CREDENTIAL_SENTINEL.to_owned(),
        target.stored_path.clone(),
        decoy.stored_path.clone(),
        "gpt-5.6".to_owned(),
    ]
}

async fn protected_row_count(pool: &SqlitePool) -> i64 {
    let row = sqlx::query(
        "SELECT (SELECT COUNT(*) FROM conversations) AS conversations, (SELECT COUNT(*) FROM messages) AS messages, (SELECT COUNT(*) FROM annotations) AS annotations, (SELECT COUNT(*) FROM index_search_chunks) AS chunks, (SELECT COUNT(*) FROM index_corrections) AS corrections",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    [
        "conversations",
        "messages",
        "annotations",
        "chunks",
        "corrections",
    ]
    .into_iter()
    .map(|column| row.get::<i64, _>(column))
    .sum()
}

async fn assert_database_counts(pool: &SqlitePool, expected: (i64, i64, i64)) {
    let conversations = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM conversations")
        .fetch_one(pool)
        .await
        .unwrap();
    let messages = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM messages")
        .fetch_one(pool)
        .await
        .unwrap();
    let annotations = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM annotations")
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!((conversations, messages, annotations), expected);
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}
