use std::sync::Arc;

use async_trait::async_trait;
use sqlx::{Row, SqlitePool};
use textbooklens_lib::{
    db::{
        Database,
        annotations::{list_annotation_markers, update_ai_annotation_summary},
        conversations::{
            BookFollowupCompletion, DeleteBookConversation, LearningPersistenceFaultInjector,
            LearningPersistenceStep, LearningRepository, NewBookQuestionCompletion,
            NewSelectionCompletion, load_book_conversation,
        },
        messages::CompletedAssistantMessage,
    },
    domain::{
        Citation, CitationReviewStatus, ContentAnchor, ContentSource, DocumentLocator,
        LearningAction, SelectionAnchor, TextQuote,
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
    initial_profile_id: Uuid,
}

impl Fixture {
    fn new() -> Self {
        let temporary = tempfile::tempdir().unwrap();
        let database = Database::open(temporary.path().join("book-learning.sqlite3")).unwrap();
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
            initial_profile_id: Uuid::new_v4(),
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

    fn new_completion(&self) -> NewBookQuestionCompletion {
        NewBookQuestionCompletion {
            book_id: self.book_id,
            question: "How do the main ideas in this textbook fit together?".to_owned(),
            assistant: CompletedAssistantMessage {
                provider_profile_id: self.initial_profile_id,
                model_id: "captured-book-model-v1".to_owned(),
                answer: "The durable book answer uses the source. [TL-C1]".to_owned(),
                available_citations: vec![self.citation()],
            },
        }
    }

    fn followup(
        &self,
        conversation_id: Uuid,
        expected_next_ordinal: u32,
        suffix: &str,
    ) -> BookFollowupCompletion {
        BookFollowupCompletion {
            book_id: self.book_id,
            conversation_id,
            expected_next_ordinal,
            question: format!("Book follow-up {suffix}?"),
            assistant: CompletedAssistantMessage {
                provider_profile_id: Uuid::new_v4(),
                model_id: format!("captured-current-model-{suffix}"),
                answer: format!("Book follow-up answer {suffix}. [TL-C1]"),
                available_citations: vec![self.citation()],
            },
        }
    }
}

#[test]
fn new_book_completion_is_one_transaction_without_annotation_or_marker() {
    let fixture = Fixture::new();
    tauri::async_runtime::block_on(async {
        let result = fixture
            .repository()
            .persist_new_book_question(fixture.new_completion())
            .await
            .unwrap();
        assert_eq!(result.annotation_id, None);
        assert!(!format!("{result:?}").contains(&result.conversation_id.to_string()));

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
        assert_eq!(conversation.get::<String, _>("scope"), "book");
        for column in ["section_id", "anchor_kind", "anchor_json", "selected_text"] {
            assert_eq!(conversation.get::<Option<String>, _>(column), None);
        }
        assert_canonical_utc(&conversation.get::<String, _>("created_at"));
        assert_canonical_utc(&conversation.get::<String, _>("updated_at"));

        let rows = sqlx::query(
            "SELECT ordinal, role, action, content, provider_id, model_id, citations_json FROM messages WHERE conversation_id = ? ORDER BY ordinal",
        )
        .bind(result.conversation_id.to_string())
        .fetch_all(fixture.pool())
        .await
        .unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].get::<i64, _>("ordinal"), 0);
        assert_eq!(rows[0].get::<String, _>("role"), "user");
        assert_eq!(rows[0].get::<String, _>("action"), "ask");
        assert_eq!(rows[0].get::<Option<String>, _>("provider_id"), None);
        assert_eq!(rows[0].get::<Option<String>, _>("model_id"), None);
        assert_eq!(rows[0].get::<Option<String>, _>("citations_json"), None);
        assert_eq!(rows[1].get::<i64, _>("ordinal"), 1);
        assert_eq!(rows[1].get::<String, _>("role"), "assistant");
        assert_eq!(rows[1].get::<String, _>("action"), "ask");
        assert_eq!(
            rows[1].get::<String, _>("provider_id"),
            fixture.initial_profile_id.to_string()
        );
        assert_eq!(
            rows[1].get::<String, _>("model_id"),
            "captured-book-model-v1"
        );
        assert_eq!(
            serde_json::from_str::<Vec<Citation>>(&rows[1].get::<String, _>("citations_json"))
                .unwrap(),
            vec![fixture.citation()]
        );

        assert_eq!(count(fixture.pool(), "conversations").await, 1);
        assert_eq!(count(fixture.pool(), "messages").await, 2);
        assert_eq!(count(fixture.pool(), "annotations").await, 0);
        assert!(
            list_annotation_markers(fixture.pool(), fixture.book_id)
                .await
                .unwrap()
                .is_empty()
        );

        let history =
            load_book_conversation(fixture.pool(), fixture.book_id, result.conversation_id)
                .await
                .unwrap();
        assert_eq!(history.id, result.conversation_id);
        assert_eq!(history.book_id, fixture.book_id);
        assert_eq!(history.messages.len(), 2);
        assert_eq!(
            history.messages[1].provider_id,
            Some(fixture.initial_profile_id)
        );
        assert_eq!(
            history.messages[1].model_id.as_deref(),
            Some("captured-book-model-v1")
        );
        assert_eq!(history.messages[1].citations, vec![fixture.citation()]);
        let debug = format!("{history:?}");
        for forbidden in [
            "How do the main ideas",
            "durable book answer",
            "captured-book-model-v1",
            &fixture.initial_profile_id.to_string(),
        ] {
            assert!(!debug.contains(forbidden));
        }
    });
}

#[test]
fn book_followup_is_cas_scoped_and_preserves_historical_provider_model() {
    let fixture = Fixture::new();
    tauri::async_runtime::block_on(async {
        let initial = fixture
            .repository()
            .persist_new_book_question(fixture.new_completion())
            .await
            .unwrap();
        let left_repository = fixture.repository();
        let right_repository = fixture.repository();
        let left = left_repository.persist_book_followup(fixture.followup(
            initial.conversation_id,
            2,
            "left",
        ));
        let right = right_repository.persist_book_followup(fixture.followup(
            initial.conversation_id,
            2,
            "right",
        ));
        let (left, right) = tokio::join!(left, right);
        assert_eq!(
            [&left, &right]
                .into_iter()
                .filter(|value| value.is_ok())
                .count(),
            1
        );
        assert_eq!(
            [&left, &right]
                .into_iter()
                .filter_map(|value| value.as_ref().err())
                .map(|error| error.code)
                .collect::<Vec<_>>(),
            [AppErrorCode::RequestConflict]
        );

        let history =
            load_book_conversation(fixture.pool(), fixture.book_id, initial.conversation_id)
                .await
                .unwrap();
        assert_eq!(history.messages.len(), 4);
        assert_eq!(history.messages[0].action, LearningAction::Ask);
        assert_eq!(history.messages[1].action, LearningAction::Ask);
        assert_eq!(history.messages[2].action, LearningAction::Continue);
        assert_eq!(history.messages[3].action, LearningAction::Continue);
        assert_eq!(
            history.messages[1].provider_id,
            Some(fixture.initial_profile_id)
        );
        assert_eq!(
            history.messages[1].model_id.as_deref(),
            Some("captured-book-model-v1")
        );
        assert!(
            history.messages[3]
                .model_id
                .as_deref()
                .is_some_and(|model| model.starts_with("captured-current-model-"))
        );

        let wrong_book = BookFollowupCompletion {
            book_id: fixture.decoy_book_id,
            conversation_id: initial.conversation_id,
            expected_next_ordinal: 4,
            question: "Cross-book follow-up?".to_owned(),
            assistant: CompletedAssistantMessage {
                provider_profile_id: Uuid::new_v4(),
                model_id: "cross-book-model".to_owned(),
                answer: "Cross-book answer.".to_owned(),
                available_citations: Vec::new(),
            },
        };
        assert_eq!(
            fixture
                .repository()
                .persist_book_followup(wrong_book)
                .await
                .unwrap_err()
                .code,
            AppErrorCode::NotFound
        );

        let selection = fixture
            .repository()
            .persist_new_selection(selection_completion(&fixture))
            .await
            .unwrap();
        let mut selection_as_book = fixture.followup(selection.conversation_id, 2, "selection");
        selection_as_book.assistant.available_citations.clear();
        selection_as_book.assistant.answer = "Must reject selection scope.".to_owned();
        assert_eq!(
            fixture
                .repository()
                .persist_book_followup(selection_as_book)
                .await
                .unwrap_err()
                .code,
            AppErrorCode::NotFound
        );

        assert_eq!(
            load_book_conversation(
                fixture.pool(),
                fixture.decoy_book_id,
                initial.conversation_id,
            )
            .await
            .unwrap_err()
            .code,
            AppErrorCode::NotFound
        );
        assert_eq!(
            load_book_conversation(fixture.pool(), fixture.book_id, selection.conversation_id,)
                .await
                .unwrap_err()
                .code,
            AppErrorCode::NotFound
        );
    });
}

#[test]
fn book_citation_validation_and_every_persistence_fault_roll_back() {
    let fixture = Fixture::new();
    tauri::async_runtime::block_on(async {
        let mut unknown = fixture.new_completion();
        unknown.assistant.answer = "Unknown citation [TL-C9]".to_owned();
        assert_invalid_new(&fixture, unknown).await;

        let mut cross_book = fixture.new_completion();
        cross_book.assistant.available_citations = vec![citation(
            fixture.decoy_book_id,
            fixture.decoy_section_id,
            1,
            "TL-C1",
        )];
        assert_invalid_new(&fixture, cross_book).await;

        let mut stale_locator = fixture.new_completion();
        stale_locator.assistant.available_citations[0].locator =
            DocumentLocator::pdf(2, 2, None).unwrap();
        assert_invalid_new(&fixture, stale_locator).await;

        let mut forbidden_source = fixture.new_completion();
        forbidden_source.assistant.available_citations[0].source = ContentSource::AiDescription;
        forbidden_source.assistant.available_citations[0].quoteable = false;
        assert_invalid_new(&fixture, forbidden_source).await;

        for step in [
            LearningPersistenceStep::ConversationInsert,
            LearningPersistenceStep::UserMessageInsert,
            LearningPersistenceStep::AssistantMessageInsert,
            LearningPersistenceStep::Commit,
        ] {
            let repository = LearningRepository::with_fault_injector(
                fixture.pool().clone(),
                Arc::new(FailAt(step)),
            );
            assert_eq!(
                repository
                    .persist_new_book_question(fixture.new_completion())
                    .await
                    .unwrap_err()
                    .code,
                AppErrorCode::DatabaseError,
                "{step:?}"
            );
            assert_counts(fixture.pool(), (0, 0, 0)).await;
        }

        let initial = fixture
            .repository()
            .persist_new_book_question(fixture.new_completion())
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
                    .persist_book_followup(fixture.followup(initial.conversation_id, 2, "fault"))
                    .await
                    .unwrap_err()
                    .code,
                AppErrorCode::DatabaseError,
                "{step:?}"
            );
            assert_counts(fixture.pool(), (1, 2, 0)).await;
        }
    });
}

#[test]
fn book_delete_is_atomic_and_does_not_touch_selection_marker() {
    let fixture = Fixture::new();
    tauri::async_runtime::block_on(async {
        let book = fixture
            .repository()
            .persist_new_book_question(fixture.new_completion())
            .await
            .unwrap();
        let selection = fixture
            .repository()
            .persist_new_selection(selection_completion(&fixture))
            .await
            .unwrap();

        for step in [
            LearningPersistenceStep::ConversationLock,
            LearningPersistenceStep::ConversationDelete,
            LearningPersistenceStep::Commit,
        ] {
            let repository = LearningRepository::with_fault_injector(
                fixture.pool().clone(),
                Arc::new(FailAt(step)),
            );
            assert_eq!(
                repository
                    .delete_book_conversation(DeleteBookConversation {
                        book_id: fixture.book_id,
                        conversation_id: book.conversation_id,
                    })
                    .await
                    .unwrap_err()
                    .code,
                AppErrorCode::DatabaseError,
                "{step:?}"
            );
            assert_counts(fixture.pool(), (2, 4, 1)).await;
        }

        fixture
            .repository()
            .delete_book_conversation(DeleteBookConversation {
                book_id: fixture.book_id,
                conversation_id: book.conversation_id,
            })
            .await
            .unwrap();
        assert_counts(fixture.pool(), (1, 2, 1)).await;
        let markers = list_annotation_markers(fixture.pool(), fixture.book_id)
            .await
            .unwrap();
        assert_eq!(markers.len(), 1);
        assert_eq!(markers[0].id, selection.annotation_id);
        assert_eq!(markers[0].conversation_id, Some(selection.conversation_id));
        assert_eq!(
            load_book_conversation(fixture.pool(), fixture.book_id, book.conversation_id)
                .await
                .unwrap_err()
                .code,
            AppErrorCode::NotFound
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sections WHERE book_id = ?")
                .bind(fixture.book_id.to_string())
                .fetch_one(fixture.pool())
                .await
                .unwrap(),
            1
        );
    });
}

#[test]
fn ai_marker_sequence_default_summary_and_revision_edit_are_durable() {
    let fixture = Fixture::new();
    tauri::async_runtime::block_on(async {
        let first = fixture
            .repository()
            .persist_new_selection(selection_completion(&fixture))
            .await
            .unwrap();
        let mut second_completion = selection_completion(&fixture);
        second_completion.question = "Explain the second selection.".to_owned();
        second_completion.assistant.answer =
            "Second compact explanation. Additional detail is not the brief.".to_owned();
        let second = fixture
            .repository()
            .persist_new_selection(second_completion)
            .await
            .unwrap();

        let markers = list_annotation_markers(fixture.pool(), fixture.book_id)
            .await
            .unwrap();
        assert_eq!(markers.len(), 2);
        assert_eq!(markers[0].id, first.annotation_id);
        assert_eq!(markers[0].sequence, Some(1));
        assert_eq!(markers[0].summary_text, "Selection answer.");
        assert_eq!(markers[1].id, second.annotation_id);
        assert_eq!(markers[1].sequence, Some(2));
        assert_eq!(markers[1].summary_text, "Second compact explanation.");
        assert_eq!(markers[1].revision, 1);
        assert!(!format!("{:?}", markers[1]).contains("Second compact"));

        update_ai_annotation_summary(
            fixture.pool(),
            fixture.book_id,
            second.annotation_id,
            markers[1].revision,
            "  User-edited brief  ".to_owned(),
        )
        .await
        .unwrap();
        let stale = update_ai_annotation_summary(
            fixture.pool(),
            fixture.book_id,
            second.annotation_id,
            markers[1].revision,
            "Stale overwrite".to_owned(),
        )
        .await
        .unwrap_err();
        assert_eq!(stale.code, AppErrorCode::RequestConflict);

        let edited = list_annotation_markers(fixture.pool(), fixture.book_id)
            .await
            .unwrap();
        assert_eq!(edited[1].summary_text, "User-edited brief");
        assert_eq!(edited[1].revision, 2);
    });
}

#[test]
fn book_history_followup_and_delete_reject_any_annotation_linkage() {
    let fixture = Fixture::new();
    tauri::async_runtime::block_on(async {
        let book = fixture
            .repository()
            .persist_new_book_question(fixture.new_completion())
            .await
            .unwrap();
        let corrupt_annotation_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO annotations (id, book_id, section_id, kind, anchor_json, selected_text, conversation_id, created_at, updated_at) VALUES (?, ?, ?, 'ai_conversation', '{}', 'corrupt cross-book link', ?, ?, ?)",
        )
        .bind(corrupt_annotation_id.to_string())
        .bind(fixture.decoy_book_id.to_string())
        .bind(fixture.decoy_section_id.to_string())
        .bind(book.conversation_id.to_string())
        .bind(FIXTURE_TIME)
        .bind(FIXTURE_TIME)
        .execute(fixture.pool())
        .await
        .unwrap();

        assert_eq!(
            load_book_conversation(fixture.pool(), fixture.book_id, book.conversation_id)
                .await
                .unwrap_err()
                .code,
            AppErrorCode::DatabaseError
        );
        assert_eq!(
            fixture
                .repository()
                .persist_book_followup(fixture.followup(book.conversation_id, 2, "corrupt"))
                .await
                .unwrap_err()
                .code,
            AppErrorCode::DatabaseError
        );
        assert_eq!(
            fixture
                .repository()
                .delete_book_conversation(DeleteBookConversation {
                    book_id: fixture.book_id,
                    conversation_id: book.conversation_id,
                })
                .await
                .unwrap_err()
                .code,
            AppErrorCode::DatabaseError
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM conversations WHERE id = ?")
                .bind(book.conversation_id.to_string())
                .fetch_one(fixture.pool())
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM annotations WHERE id = ?")
                .bind(corrupt_annotation_id.to_string())
                .fetch_one(fixture.pool())
                .await
                .unwrap(),
            1
        );
    });
}

fn selection_completion(fixture: &Fixture) -> NewSelectionCompletion {
    let selected = "Selection conversation remains independent.";
    NewSelectionCompletion {
        book_id: fixture.book_id,
        section_id: fixture.section_id,
        anchor: text_anchor(fixture.section_id, selected),
        selected_text: Some(selected.to_owned()),
        action: LearningAction::Explain,
        question: "Explain this selection.".to_owned(),
        assistant: CompletedAssistantMessage {
            provider_profile_id: Uuid::new_v4(),
            model_id: "selection-model".to_owned(),
            answer: "Selection answer.".to_owned(),
            available_citations: Vec::new(),
        },
    }
}

fn text_anchor(section_id: Uuid, exact: &str) -> ContentAnchor {
    ContentAnchor::Text {
        selection: SelectionAnchor {
            locator: DocumentLocator::pdf(1, 1, None).unwrap(),
            quote: TextQuote::new(exact.to_owned(), String::new(), String::new()).unwrap(),
            section_id: Some(section_id),
        },
    }
}

fn citation(book_id: Uuid, section_id: Uuid, page: u32, id: &str) -> Citation {
    Citation::new(
        id.to_owned(),
        format!("Page {page}"),
        book_id,
        Some(section_id),
        DocumentLocator::pdf(page, page, None).unwrap(),
        ContentSource::LocalText,
        CitationReviewStatus::NotRequired,
    )
    .unwrap()
}

async fn seed_pdf(pool: &SqlitePool, book_id: Uuid, section_id: Uuid, suffix: &str) {
    sqlx::query(
        "INSERT INTO books (id, sha256, title, format, original_filename, stored_path, import_status, created_at, updated_at) VALUES (?, ?, ?, 'pdf', ?, ?, 'ready', ?, ?)",
    )
    .bind(book_id.to_string())
    .bind(suffix.repeat(64))
    .bind(format!("Book {suffix}"))
    .bind(format!("book-{suffix}.pdf"))
    .bind(format!("books/{suffix}/original.pdf"))
    .bind(FIXTURE_TIME)
    .bind(FIXTURE_TIME)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO sections (id, book_id, ordinal, title, locator_json) VALUES (?, ?, 0, 'Section', ?)",
    )
    .bind(section_id.to_string())
    .bind(book_id.to_string())
    .bind(serde_json::to_string(&DocumentLocator::pdf(1, 1, None).unwrap()).unwrap())
    .execute(pool)
    .await
    .unwrap();
}

async fn assert_invalid_new(fixture: &Fixture, input: NewBookQuestionCompletion) {
    assert_eq!(
        fixture
            .repository()
            .persist_new_book_question(input)
            .await
            .unwrap_err()
            .code,
        AppErrorCode::InvalidInput
    );
    assert_counts(fixture.pool(), (0, 0, 0)).await;
}

async fn count(pool: &SqlitePool, table: &str) -> i64 {
    let query = match table {
        "conversations" => "SELECT COUNT(*) FROM conversations",
        "messages" => "SELECT COUNT(*) FROM messages",
        "annotations" => "SELECT COUNT(*) FROM annotations",
        _ => panic!("unsupported test table"),
    };
    sqlx::query_scalar(query).fetch_one(pool).await.unwrap()
}

async fn assert_counts(pool: &SqlitePool, expected: (i64, i64, i64)) {
    assert_eq!(
        (
            count(pool, "conversations").await,
            count(pool, "messages").await,
            count(pool, "annotations").await,
        ),
        expected
    );
}

fn assert_canonical_utc(value: &str) {
    let parsed = chrono::DateTime::parse_from_rfc3339(value).unwrap();
    assert_eq!(
        parsed.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        value
    );
}

#[derive(Clone, Copy)]
struct FailAt(LearningPersistenceStep);

#[async_trait]
impl LearningPersistenceFaultInjector for FailAt {
    async fn checkpoint(&self, step: LearningPersistenceStep) -> AppResult<()> {
        if step == self.0 {
            Err(AppError::new(AppErrorCode::DatabaseError))
        } else {
            Ok(())
        }
    }
}
