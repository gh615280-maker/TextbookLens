use crate::domain::{ContentAnchor, DocumentLocator, SelectionAnchor, TextQuote};

use super::*;

fn text_anchor(section_id: Uuid, text: &str) -> ContentAnchor {
    ContentAnchor::Text {
        selection: SelectionAnchor {
            locator: DocumentLocator::pdf(1, 1, None).unwrap(),
            quote: TextQuote::new(text.to_owned(), String::new(), String::new()).unwrap(),
            section_id: Some(section_id),
        },
    }
}

fn completion(section_id: Uuid) -> NewSelectionCompletion {
    NewSelectionCompletion {
        book_id: Uuid::new_v4(),
        section_id,
        anchor: text_anchor(section_id, "private-selection-sentinel"),
        selected_text: Some("private-selection-sentinel".to_owned()),
        action: LearningAction::Explain,
        question: "private-question-sentinel".to_owned(),
        assistant: CompletedAssistantMessage {
            provider_profile_id: Uuid::new_v4(),
            model_id: "synthetic-model".to_owned(),
            answer: "private-answer-sentinel".to_owned(),
            available_citations: Vec::new(),
        },
    }
}

#[test]
fn conversations_validate_immutable_selection_snapshot_and_redact_debug() {
    let section_id = Uuid::new_v4();
    let input = completion(section_id);
    assert!(validate_new_selection(&input).is_ok());
    let debug = format!("{input:?}");
    for sentinel in [
        "private-selection-sentinel",
        "private-question-sentinel",
        "private-answer-sentinel",
        "synthetic-model",
    ] {
        assert!(!debug.contains(sentinel));
    }

    let mut mismatched = completion(section_id);
    mismatched.selected_text = Some("changed".to_owned());
    assert_eq!(
        validate_new_selection(&mismatched).unwrap_err().code,
        AppErrorCode::InvalidInput
    );
    let mut wrong_action = completion(section_id);
    wrong_action.action = LearningAction::Continue;
    assert_eq!(
        validate_new_selection(&wrong_action).unwrap_err().code,
        AppErrorCode::InvalidInput
    );
}

#[test]
fn conversations_history_anchor_validation_rejects_cross_format_and_cross_section_data() {
    let section_id = Uuid::new_v4();
    let anchor = text_anchor(section_id, "synthetic quote");
    assert!(history_anchor_is_structurally_valid(
        &anchor, section_id, "pdf"
    ));
    assert!(!history_anchor_is_structurally_valid(
        &anchor, section_id, "epub"
    ));
    assert!(!history_anchor_is_structurally_valid(
        &anchor,
        Uuid::new_v4(),
        "pdf"
    ));

    let ContentAnchor::Text { mut selection } = anchor else {
        unreachable!();
    };
    selection.quote.prefix = "x".repeat(65);
    assert!(!history_anchor_is_structurally_valid(
        &ContentAnchor::Text { selection },
        section_id,
        "pdf"
    ));
}
