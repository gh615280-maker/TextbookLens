use uuid::Uuid;

use crate::{
    ai::registry::ProviderCapabilityRegistry,
    db::{
        Database,
        conversations::{LearningRepository, NewSelectionCompletion},
        messages::CompletedAssistantMessage,
    },
    domain::{
        ContentAnchor, DocumentLocator, LearningAction, SelectionAnchor, TextQuote, UnifiedMessage,
        UnifiedRole,
    },
};

use super::{
    super::registry::LearningPersistenceTarget, prepare_conversation_followup,
    retain_recent_history_pairs, strip_historical_citation_ids,
};

const FIXTURE_TIME: &str = "2026-08-05T00:00:00.000Z";

#[test]
fn history_compression_keeps_only_complete_recent_pairs_within_budget() {
    let history = vec![
        message(UnifiedRole::User, "old user"),
        message(UnifiedRole::Assistant, "old assistant"),
        message(UnifiedRole::User, "recent user"),
        message(UnifiedRole::Assistant, "recent assistant"),
    ];
    let recent_pair_cost = 32 + "recent user".chars().count() + "recent assistant".chars().count();
    let retained = retain_recent_history_pairs(history, recent_pair_cost as u64);
    assert_eq!(retained.len(), 2);
    assert_eq!(retained[0].content, "recent user");
    assert_eq!(retained[1].content, "recent assistant");
}

#[test]
fn history_removes_prior_request_citation_ids_without_touching_unicode() {
    assert_eq!(
        strip_historical_citation_ids("教材 TL-C1 与 TL-C23；保留 TL-Cx。"),
        "教材 [historical citation omitted] 与 [historical citation omitted]；保留 TL-Cx。"
    );
}

#[test]
fn history_loads_current_profile_instruction_and_only_bounded_same_conversation_pairs() {
    let temporary = tempfile::tempdir().unwrap();
    let database = Database::open(temporary.path().join("history.sqlite3")).unwrap();
    let book_id = Uuid::new_v4();
    let section_id = Uuid::new_v4();
    let profile_id = Uuid::new_v4();
    tauri::async_runtime::block_on(async {
        seed(database.pool(), book_id, section_id, profile_id).await;
        let persisted = LearningRepository::new(database.pool().clone())
            .persist_new_selection(NewSelectionCompletion {
                book_id,
                section_id,
                anchor: text_anchor(section_id, "immutable selection"),
                selected_text: Some("immutable selection".to_owned()),
                action: LearningAction::Explain,
                question: "Explain this selection.".to_owned(),
                assistant: CompletedAssistantMessage {
                    provider_profile_id: profile_id,
                    model_id: "gpt-5.6".to_owned(),
                    answer: "Initial answer.".to_owned(),
                    available_citations: Vec::new(),
                },
            })
            .await
            .unwrap();
        for ordinal in 2..72i64 {
            let role = if ordinal % 2 == 0 {
                "user"
            } else {
                "assistant"
            };
            let content = format!("history-{ordinal}-{}", "x".repeat(180));
            sqlx::query(
                "INSERT INTO messages (id, conversation_id, ordinal, role, action, content, provider_id, model_id, citations_json, created_at) VALUES (?, ?, ?, ?, 'continue', ?, ?, ?, ?, ?)",
            )
            .bind(Uuid::new_v4().to_string())
            .bind(persisted.conversation_id.to_string())
            .bind(ordinal)
            .bind(role)
            .bind(content)
            .bind((role == "assistant").then(|| profile_id.to_string()))
            .bind((role == "assistant").then_some("gpt-5.6"))
            .bind((role == "assistant").then_some("[]"))
            .bind(FIXTURE_TIME)
            .execute(database.pool())
            .await
            .unwrap();
        }

        let prepared = prepare_conversation_followup(
            database.pool(),
            &ProviderCapabilityRegistry::load_embedded().unwrap(),
            persisted.conversation_id,
            "What should I study next?".to_owned(),
        )
        .await
        .unwrap();
        assert_eq!(prepared.context.provider_profile_id, profile_id);
        assert_eq!(prepared.context.model_id, "gpt-5.6");
        assert!(
            prepared
                .chat_request
                .system
                .contains("Use a compact worked example.")
        );
        assert!(prepared.chat_request.messages.len() < 65);
        assert_eq!(
            prepared.chat_request.messages.last().unwrap().content,
            "What should I study next?"
        );
        let LearningPersistenceTarget::SelectionFollowup(target) = &prepared.context.target else {
            panic!("followup target");
        };
        assert_eq!(target.book_id, book_id);
        assert_eq!(target.conversation_id, persisted.conversation_id);
        assert_eq!(target.expected_next_ordinal, 72);
    });
}

fn message(role: UnifiedRole, content: &str) -> UnifiedMessage {
    UnifiedMessage {
        role,
        content: content.to_owned(),
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

async fn seed(pool: &sqlx::SqlitePool, book_id: Uuid, section_id: Uuid, profile_id: Uuid) {
    sqlx::query(
        "INSERT INTO books (id, sha256, title, format, original_filename, stored_path, import_status, created_at, updated_at) VALUES (?, ?, 'History book', 'pdf', 'history.pdf', 'books/history/original.pdf', 'ready', ?, ?)",
    )
    .bind(book_id.to_string())
    .bind("b".repeat(64))
    .bind(FIXTURE_TIME)
    .bind(FIXTURE_TIME)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO sections (id, book_id, ordinal, title, locator_json) VALUES (?, ?, 0, 'History section', ?)",
    )
    .bind(section_id.to_string())
    .bind(book_id.to_string())
    .bind(serde_json::to_string(&DocumentLocator::pdf(1, 1, None).unwrap()).unwrap())
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO provider_profiles (id, provider_kind, display_name, model_id, context_window_tokens, is_active, created_at, updated_at, validated_at) VALUES (?, 'openai', 'Current profile', 'gpt-5.6', 8000, 1, ?, ?, ?)",
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
        "UPDATE teaching_preferences SET instruction = 'Use a compact worked example.', revision = 1, updated_at = ? WHERE id = 1",
    )
    .bind(FIXTURE_TIME)
    .execute(pool)
    .await
    .unwrap();
}
