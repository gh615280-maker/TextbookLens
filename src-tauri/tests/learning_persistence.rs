use std::sync::Arc;

use async_trait::async_trait;
use sqlx::{Row, SqlitePool};
use textbooklens_lib::{
    db::{
        Database,
        annotations::list_annotation_markers,
        conversations::{
            DeleteSelectionConversation, FollowupCompletion, LearningPersistenceFaultInjector,
            LearningPersistenceStep, LearningRepository, NewSelectionCompletion,
            PersistedLearningResult, load_selection_conversation,
        },
        messages::CompletedAssistantMessage,
    },
    domain::{
        Citation, CitationReviewStatus, ContentAnchor, ContentSource, DocumentLocator,
        LearningAction, NormalizedRect, RegionAnchor, RegionLocator, SelectionAnchor, TextQuote,
    },
    errors::{AppError, AppErrorCode, AppResult},
};
use uuid::Uuid;

const FIXTURE_TIME: &str = "2026-08-05T00:00:00.000Z";

struct Fixture {
    _temporary: tempfile::TempDir,
    database: Database,
    book_id: Uuid,
    section_id: Uuid,
    decoy_book_id: Uuid,
    decoy_section_id: Uuid,
    profile_id: Uuid,
}

impl Fixture {
    fn new() -> Self {
        let temporary = tempfile::tempdir().unwrap();
        let database = Database::open(temporary.path().join("learning.sqlite3")).unwrap();
        let book_id = Uuid::new_v4();
        let section_id = Uuid::new_v4();
        let decoy_book_id = Uuid::new_v4();
        let decoy_section_id = Uuid::new_v4();
        tauri::async_runtime::block_on(async {
            seed_pdf(database.pool(), book_id, section_id, "a").await;
            seed_pdf(database.pool(), decoy_book_id, decoy_section_id, "b").await;
        });
        Self {
            _temporary: temporary,
            database,
            book_id,
            section_id,
            decoy_book_id,
            decoy_section_id,
            profile_id: Uuid::new_v4(),
        }
    }

    fn pool(&self) -> &SqlitePool {
        self.database.pool()
    }

    fn repository(&self) -> LearningRepository {
        LearningRepository::new(self.pool().clone())
    }

    fn citation(&self) -> Citation {
        citation(self.book_id, self.section_id, 1, "TL-C1")
    }

    fn new_completion(&self) -> NewSelectionCompletion {
        let selected = "线性代数 immutable selection";
        NewSelectionCompletion {
            book_id: self.book_id,
            section_id: self.section_id,
            anchor: text_anchor(self.section_id, selected),
            selected_text: Some(selected.to_owned()),
            action: LearningAction::Explain,
            question: "Explain the selected textbook content.".to_owned(),
            assistant: CompletedAssistantMessage {
                provider_profile_id: self.profile_id,
                model_id: "captured-model-v1".to_owned(),
                answer: "完成回答 [TL-C1]".to_owned(),
                available_citations: vec![self.citation()],
            },
        }
    }

    fn followup(
        &self,
        conversation_id: Uuid,
        expected_next_ordinal: u32,
        suffix: &str,
    ) -> FollowupCompletion {
        FollowupCompletion {
            book_id: self.book_id,
            conversation_id,
            expected_next_ordinal,
            question: format!("Follow up {suffix}?"),
            assistant: CompletedAssistantMessage {
                provider_profile_id: Uuid::new_v4(),
                model_id: format!("captured-followup-model-{suffix}"),
                answer: format!("Followup answer {suffix} [TL-C1]"),
                available_citations: vec![self.citation()],
            },
        }
    }
}

#[test]
fn completed_new_selection_is_one_exact_transaction_with_immutable_metadata() {
    let fixture = Fixture::new();
    tauri::async_runtime::block_on(async {
        let result = fixture
            .repository()
            .persist_new_selection(fixture.new_completion())
            .await
            .unwrap();
        assert_atomic_shape(fixture.pool(), result).await;

        let conversation = sqlx::query(
            "SELECT book_id, section_id, scope, anchor_kind, anchor_json, selected_text, created_at, updated_at FROM conversations WHERE id = ?",
        )
        .bind(result.conversation_id.to_string())
        .fetch_one(fixture.pool())
        .await
        .unwrap();
        assert_eq!(
            conversation.get::<String, _>("book_id"),
            fixture.book_id.to_string()
        );
        assert_eq!(
            conversation.get::<String, _>("section_id"),
            fixture.section_id.to_string()
        );
        assert_eq!(conversation.get::<String, _>("scope"), "selection");
        assert_eq!(conversation.get::<String, _>("anchor_kind"), "text");
        assert_eq!(
            conversation.get::<String, _>("selected_text"),
            "线性代数 immutable selection"
        );
        let stored_anchor: ContentAnchor =
            serde_json::from_str(&conversation.get::<String, _>("anchor_json")).unwrap();
        assert_eq!(
            stored_anchor,
            text_anchor(fixture.section_id, "线性代数 immutable selection")
        );
        assert_canonical_utc(&conversation.get::<String, _>("created_at"));
        assert_canonical_utc(&conversation.get::<String, _>("updated_at"));

        let rows = sqlx::query(
            "SELECT ordinal, role, action, content, provider_id, model_id, citations_json, created_at FROM messages WHERE conversation_id = ? ORDER BY ordinal",
        )
        .bind(result.conversation_id.to_string())
        .fetch_all(fixture.pool())
        .await
        .unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].get::<i64, _>("ordinal"), 0);
        assert_eq!(rows[0].get::<String, _>("role"), "user");
        assert_eq!(rows[0].get::<String, _>("action"), "explain");
        assert_eq!(
            rows[0].get::<String, _>("content"),
            "Explain the selected textbook content."
        );
        assert_eq!(rows[0].get::<Option<String>, _>("provider_id"), None);
        assert_eq!(rows[0].get::<Option<String>, _>("model_id"), None);
        assert_eq!(rows[0].get::<Option<String>, _>("citations_json"), None);
        assert_eq!(rows[1].get::<i64, _>("ordinal"), 1);
        assert_eq!(rows[1].get::<String, _>("role"), "assistant");
        assert_eq!(rows[1].get::<String, _>("action"), "explain");
        assert_eq!(
            rows[1].get::<String, _>("provider_id"),
            fixture.profile_id.to_string()
        );
        assert_eq!(rows[1].get::<String, _>("model_id"), "captured-model-v1");
        let citations: Vec<Citation> =
            serde_json::from_str(&rows[1].get::<String, _>("citations_json")).unwrap();
        assert_eq!(citations, vec![fixture.citation()]);
        assert_canonical_utc(&rows[0].get::<String, _>("created_at"));
        assert_canonical_utc(&rows[1].get::<String, _>("created_at"));
    });
}

#[test]
fn annotations_marker_dto_exposes_only_anchor_type_ids_and_safe_label() {
    let fixture = Fixture::new();
    tauri::async_runtime::block_on(async {
        let result = fixture
            .repository()
            .persist_new_selection(fixture.new_completion())
            .await
            .unwrap();
        let markers = list_annotation_markers(fixture.pool(), fixture.book_id)
            .await
            .unwrap();
        assert_eq!(markers.len(), 1);
        assert_eq!(markers[0].conversation_id, Some(result.conversation_id));
        assert_eq!(
            markers[0].accessibility_label,
            "View AI conversation marker"
        );
        let serialized = serde_json::to_string(&markers).unwrap();
        for forbidden in [
            "private-answer-sentinel",
            "Explain the selected textbook content.",
            "captured-model-v1",
            "credential",
            "provider_payload",
            "hidden_reasoning",
        ] {
            assert!(!serialized.contains(forbidden));
        }
        let debug = format!("{:?}", markers[0]);
        assert!(debug.contains("<redacted>"));
        assert!(!debug.contains("线性代数 immutable selection"));
    });
}

#[test]
fn visual_region_completion_preserves_null_selected_text_without_image_bytes() {
    let fixture = Fixture::new();
    tauri::async_runtime::block_on(async {
        let mut input = fixture.new_completion();
        input.anchor = ContentAnchor::Region {
            region: RegionAnchor::new(
                RegionLocator::pdf(1).unwrap(),
                NormalizedRect::new(0.1, 0.2, 0.3, 0.4).unwrap(),
                "c".repeat(64),
                None,
            )
            .unwrap(),
        };
        input.selected_text = None;
        let result = fixture
            .repository()
            .persist_new_selection(input)
            .await
            .unwrap();
        let row = sqlx::query(
            "SELECT anchor_kind, anchor_json, selected_text FROM conversations WHERE id = ?",
        )
        .bind(result.conversation_id.to_string())
        .fetch_one(fixture.pool())
        .await
        .unwrap();
        assert_eq!(row.get::<String, _>("anchor_kind"), "region");
        assert_eq!(row.get::<Option<String>, _>("selected_text"), None);
        let anchor_json = row.get::<String, _>("anchor_json");
        assert!(!anchor_json.contains("image"));
        assert!(!anchor_json.contains("path"));
        assert_atomic_shape(fixture.pool(), result).await;
    });
}

#[test]
fn conversations_history_load_preserves_each_assistant_provider_model_and_citations() {
    let fixture = Fixture::new();
    tauri::async_runtime::block_on(async {
        let result = fixture
            .repository()
            .persist_new_selection(fixture.new_completion())
            .await
            .unwrap();
        let followup = fixture.followup(result.conversation_id, 2, "immutable-v2");
        let followup_provider = followup.assistant.provider_profile_id;
        let followup_model = followup.assistant.model_id.clone();
        fixture
            .repository()
            .persist_followup(followup)
            .await
            .unwrap();

        let history =
            load_selection_conversation(fixture.pool(), fixture.book_id, result.conversation_id)
                .await
                .unwrap();
        assert_eq!(history.annotation_id, result.annotation_id);
        assert_eq!(history.status, "completed");
        assert_eq!(history.messages.len(), 4);
        assert_eq!(history.messages[1].provider_id, Some(fixture.profile_id));
        assert_eq!(
            history.messages[1].model_id.as_deref(),
            Some("captured-model-v1")
        );
        assert_eq!(history.messages[1].citations, vec![fixture.citation()]);
        assert_eq!(history.messages[3].provider_id, Some(followup_provider));
        assert_eq!(
            history.messages[3].model_id.as_deref(),
            Some(followup_model.as_str())
        );
        assert_eq!(history.messages[3].citations, vec![fixture.citation()]);
        let debug = format!("{history:?}");
        for secret in [
            "线性代数 immutable selection",
            "完成回答",
            "captured-model-v1",
            followup_model.as_str(),
        ] {
            assert!(!debug.contains(secret));
        }
    });
}

#[test]
fn invalid_unknown_and_cross_book_citations_roll_back_every_row() {
    let fixture = Fixture::new();
    tauri::async_runtime::block_on(async {
        let mut unknown = fixture.new_completion();
        unknown.assistant.answer = "Invented [TL-C9]".to_owned();
        assert_eq!(
            fixture
                .repository()
                .persist_new_selection(unknown)
                .await
                .unwrap_err()
                .code,
            AppErrorCode::InvalidInput
        );
        assert_counts(fixture.pool(), (0, 0, 0)).await;

        let mut empty_answer = fixture.new_completion();
        empty_answer.assistant.answer = " \n\t".to_owned();
        assert_eq!(
            fixture
                .repository()
                .persist_new_selection(empty_answer)
                .await
                .unwrap_err()
                .code,
            AppErrorCode::InvalidInput
        );
        let mut oversized_question = fixture.new_completion();
        oversized_question.question = "q".repeat(64 * 1024 + 1);
        assert_eq!(
            fixture
                .repository()
                .persist_new_selection(oversized_question)
                .await
                .unwrap_err()
                .code,
            AppErrorCode::InvalidInput
        );
        assert_counts(fixture.pool(), (0, 0, 0)).await;

        let mut decoy = fixture.new_completion();
        decoy.assistant.available_citations = vec![citation(
            fixture.decoy_book_id,
            fixture.decoy_section_id,
            1,
            "TL-C1",
        )];
        assert_eq!(
            fixture
                .repository()
                .persist_new_selection(decoy)
                .await
                .unwrap_err()
                .code,
            AppErrorCode::InvalidInput
        );
        assert_counts(fixture.pool(), (0, 0, 0)).await;

        let mut stale_locator = fixture.new_completion();
        stale_locator.assistant.available_citations[0].locator =
            DocumentLocator::pdf(2, 2, None).unwrap();
        assert_eq!(
            fixture
                .repository()
                .persist_new_selection(stale_locator)
                .await
                .unwrap_err()
                .code,
            AppErrorCode::InvalidInput
        );
        assert_counts(fixture.pool(), (0, 0, 0)).await;

        let mut forged_source = fixture.new_completion();
        forged_source.assistant.available_citations[0].source = ContentSource::AiTranscribed;
        forged_source.assistant.available_citations[0].review_status =
            CitationReviewStatus::Indexed;
        assert_eq!(
            fixture
                .repository()
                .persist_new_selection(forged_source)
                .await
                .unwrap_err()
                .code,
            AppErrorCode::InvalidInput
        );
        assert_counts(fixture.pool(), (0, 0, 0)).await;
    });
}

#[test]
fn deterministic_faults_at_every_new_insert_and_commit_leave_zero_rows() {
    let fixture = Fixture::new();
    tauri::async_runtime::block_on(async {
        for step in [
            LearningPersistenceStep::ConversationInsert,
            LearningPersistenceStep::UserMessageInsert,
            LearningPersistenceStep::AssistantMessageInsert,
            LearningPersistenceStep::AnnotationInsert,
            LearningPersistenceStep::Commit,
        ] {
            let repository = LearningRepository::with_fault_injector(
                fixture.pool().clone(),
                Arc::new(FailAt(step)),
            );
            assert_eq!(
                repository
                    .persist_new_selection(fixture.new_completion())
                    .await
                    .unwrap_err()
                    .code,
                AppErrorCode::DatabaseError,
                "{step:?}"
            );
            assert_counts(fixture.pool(), (0, 0, 0)).await;
        }
    });
}

#[test]
fn followup_lock_allows_one_same_ordinal_winner_and_isolates_decoys() {
    let fixture = Fixture::new();
    tauri::async_runtime::block_on(async {
        let result = fixture
            .repository()
            .persist_new_selection(fixture.new_completion())
            .await
            .unwrap();
        let left_repository = fixture.repository();
        let right_repository = fixture.repository();
        let left =
            left_repository.persist_followup(fixture.followup(result.conversation_id, 2, "left"));
        let right =
            right_repository.persist_followup(fixture.followup(result.conversation_id, 2, "right"));
        let (left, right) = tokio::join!(left, right);
        let outcomes = [left, right];
        assert_eq!(outcomes.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(
            outcomes
                .iter()
                .filter_map(|result| result.as_ref().err())
                .map(|error| error.code)
                .collect::<Vec<_>>(),
            [AppErrorCode::RequestConflict]
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM messages WHERE conversation_id = ?")
                .bind(result.conversation_id.to_string())
                .fetch_one(fixture.pool())
                .await
                .unwrap(),
            4
        );

        let second = fixture
            .repository()
            .persist_new_selection(fixture.new_completion())
            .await
            .unwrap();
        let first_repository = fixture.repository();
        let second_repository = fixture.repository();
        let first_followup = first_repository.persist_followup(fixture.followup(
            result.conversation_id,
            4,
            "first-independent",
        ));
        let second_followup = second_repository.persist_followup(fixture.followup(
            second.conversation_id,
            2,
            "second-independent",
        ));
        let (first_followup, second_followup) = tokio::join!(first_followup, second_followup);
        first_followup.unwrap();
        second_followup.unwrap();
        let ordinals: Vec<i64> = sqlx::query_scalar(
            "SELECT ordinal FROM messages WHERE conversation_id = ? ORDER BY ordinal",
        )
        .bind(result.conversation_id.to_string())
        .fetch_all(fixture.pool())
        .await
        .unwrap();
        assert_eq!(ordinals, [0, 1, 2, 3, 4, 5]);

        let mut cross_book = fixture.followup(result.conversation_id, 6, "decoy");
        cross_book.book_id = fixture.decoy_book_id;
        assert_eq!(
            fixture
                .repository()
                .persist_followup(cross_book)
                .await
                .unwrap_err()
                .code,
            AppErrorCode::NotFound
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM messages WHERE conversation_id = ?")
                .bind(result.conversation_id.to_string())
                .fetch_one(fixture.pool())
                .await
                .unwrap(),
            6
        );
    });
}

#[test]
fn conversations_followup_faults_roll_back_pair_and_delete_uses_trigger_once_atomically() {
    let fixture = Fixture::new();
    tauri::async_runtime::block_on(async {
        let result = fixture
            .repository()
            .persist_new_selection(fixture.new_completion())
            .await
            .unwrap();
        for step in [
            LearningPersistenceStep::ConversationLock,
            LearningPersistenceStep::UserMessageInsert,
            LearningPersistenceStep::AssistantMessageInsert,
            LearningPersistenceStep::ConversationUpdate,
            LearningPersistenceStep::Commit,
        ] {
            let repository = LearningRepository::with_fault_injector(
                fixture.pool().clone(),
                Arc::new(FailAt(step)),
            );
            assert_eq!(
                repository
                    .persist_followup(fixture.followup(result.conversation_id, 2, "fault"))
                    .await
                    .unwrap_err()
                    .code,
                AppErrorCode::DatabaseError,
                "{step:?}"
            );
            assert_eq!(
                sqlx::query_scalar::<_, i64>(
                    "SELECT COUNT(*) FROM messages WHERE conversation_id = ?",
                )
                .bind(result.conversation_id.to_string())
                .fetch_one(fixture.pool())
                .await
                .unwrap(),
                2
            );
        }

        let delete = DeleteSelectionConversation {
            book_id: fixture.book_id,
            conversation_id: result.conversation_id,
            annotation_id: result.annotation_id,
        };
        for step in [
            LearningPersistenceStep::ConversationLock,
            LearningPersistenceStep::AnnotationDelete,
            LearningPersistenceStep::Commit,
        ] {
            let repository = LearningRepository::with_fault_injector(
                fixture.pool().clone(),
                Arc::new(FailAt(step)),
            );
            assert_eq!(
                repository
                    .delete_selection_conversation(delete)
                    .await
                    .unwrap_err()
                    .code,
                AppErrorCode::DatabaseError,
                "{step:?}"
            );
            assert_counts(fixture.pool(), (1, 2, 1)).await;
        }

        let decoy_delete = DeleteSelectionConversation {
            annotation_id: Uuid::new_v4(),
            ..delete
        };
        assert_eq!(
            fixture
                .repository()
                .delete_selection_conversation(decoy_delete)
                .await
                .unwrap_err()
                .code,
            AppErrorCode::NotFound
        );
        assert_counts(fixture.pool(), (1, 2, 1)).await;

        fixture
            .repository()
            .delete_selection_conversation(delete)
            .await
            .unwrap();
        assert_counts(fixture.pool(), (0, 0, 0)).await;
    });
}

struct FailAt(LearningPersistenceStep);

#[async_trait]
impl LearningPersistenceFaultInjector for FailAt {
    async fn checkpoint(&self, step: LearningPersistenceStep) -> AppResult<()> {
        if step == self.0 {
            return Err(AppError::database(
                "synthetic learning persistence fault; content omitted",
            ));
        }
        Ok(())
    }
}

async fn assert_atomic_shape(pool: &SqlitePool, result: PersistedLearningResult) {
    assert_counts(pool, (1, 2, 1)).await;
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT conversation_id FROM annotations WHERE id = ?")
            .bind(result.annotation_id.to_string())
            .fetch_one(pool)
            .await
            .unwrap(),
        result.conversation_id.to_string()
    );
}

async fn assert_counts(pool: &SqlitePool, expected: (i64, i64, i64)) {
    let conversations: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM conversations")
        .fetch_one(pool)
        .await
        .unwrap();
    let messages: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages")
        .fetch_one(pool)
        .await
        .unwrap();
    let annotations: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM annotations")
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!((conversations, messages, annotations), expected);
}

fn assert_canonical_utc(value: &str) {
    let parsed = chrono::DateTime::parse_from_rfc3339(value).unwrap();
    assert_eq!(
        parsed.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        value
    );
}

fn text_anchor(section_id: Uuid, exact: &str) -> ContentAnchor {
    ContentAnchor::Text {
        selection: SelectionAnchor {
            locator: DocumentLocator::pdf(1, 1, None).unwrap(),
            quote: TextQuote::new(exact.to_owned(), "前".to_owned(), "后".to_owned()).unwrap(),
            section_id: Some(section_id),
        },
    }
}

fn citation(book_id: Uuid, section_id: Uuid, page: u32, id: &str) -> Citation {
    Citation::new(
        id.to_owned(),
        format!("page {page}"),
        book_id,
        Some(section_id),
        DocumentLocator::pdf(page, page, None).unwrap(),
        ContentSource::LocalText,
        CitationReviewStatus::NotRequired,
    )
    .unwrap()
}

async fn seed_pdf(pool: &SqlitePool, book_id: Uuid, section_id: Uuid, hash_prefix: &str) {
    sqlx::query(
        "INSERT INTO books (id, sha256, title, format, original_filename, stored_path, import_status, created_at, updated_at) VALUES (?, ?, 'Synthetic learning book', 'pdf', 'synthetic.pdf', 'books/synthetic/original.pdf', 'ready', ?, ?)",
    )
    .bind(book_id.to_string())
    .bind(hash_prefix.repeat(64))
    .bind(FIXTURE_TIME)
    .bind(FIXTURE_TIME)
    .execute(pool)
    .await
    .unwrap();
    let locator = serde_json::to_string(&DocumentLocator::pdf(1, 1, None).unwrap()).unwrap();
    sqlx::query(
        "INSERT INTO sections (id, book_id, ordinal, title, locator_json) VALUES (?, ?, 0, 'Section', ?)",
    )
    .bind(section_id.to_string())
    .bind(book_id.to_string())
    .bind(locator)
    .execute(pool)
    .await
    .unwrap();
}
