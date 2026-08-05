use crate::domain::{Citation, LearningAction};

use super::*;

fn assistant(answer: &str) -> CompletedAssistantMessage {
    CompletedAssistantMessage {
        provider_profile_id: Uuid::new_v4(),
        model_id: "synthetic-model".to_owned(),
        answer: answer.to_owned(),
        available_citations: Vec::<Citation>::new(),
    }
}

#[test]
fn messages_extract_only_bounded_request_local_citation_ids() {
    assert_eq!(
        extract_model_citation_ids("Answer [TL-C2], then TL-C1 and TL-C2.").unwrap(),
        ["TL-C2", "TL-C1"]
    );
    for invalid in ["TL-C0", "TL-C01", "xTL-C1", "TL-C1x", "TL-C4294967296"] {
        assert!(extract_model_citation_ids(invalid).is_err(), "{invalid}");
    }
    assert!(
        extract_model_citation_ids("TL-C without an ordinal")
            .unwrap()
            .is_empty()
    );
}

#[test]
fn messages_reject_empty_control_and_oversized_content_without_debug_leaks() {
    for value in ["", " \n\t", "question\rhidden"] {
        assert!(validate_question(value).is_err());
    }
    assert!(validate_question(&"q".repeat(MAX_LEARNING_QUESTION_BYTES + 1)).is_err());

    let mut value = assistant("private-answer-sentinel");
    assert!(validate_assistant(&value).is_ok());
    assert!(!format!("{value:?}").contains("private-answer-sentinel"));
    value.answer = String::new();
    assert!(validate_assistant(&value).is_err());
    value = assistant("answer\0hidden");
    assert!(validate_assistant(&value).is_err());
    value = assistant("answer");
    value.model_id = " model ".to_owned();
    assert!(validate_assistant(&value).is_err());
    assert_eq!(action_name(LearningAction::Continue), "continue");
}
