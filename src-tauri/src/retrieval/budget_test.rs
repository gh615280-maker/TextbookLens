use crate::{
    domain::ContextMode,
    errors::AppErrorCode,
    retrieval::budget::{
        InputBudget, LONG_CONTEXT_CAP, MINIMUM_OUTPUT_RESERVE, PROTOCOL_RESERVE,
        STANDARD_CONTEXT_CAP, conservative_rendered_count, conservative_token_count,
    },
};

#[test]
fn budget_exactly_caps_standard_long_and_reserves_output_plus_framing() {
    let standard = InputBudget::new(u32::MAX, ContextMode::Standard, 1);
    assert_eq!(standard.effective_window, STANDARD_CONTEXT_CAP);
    assert_eq!(standard.output_reserve, MINIMUM_OUTPUT_RESERVE);
    assert_eq!(standard.protocol_reserve, PROTOCOL_RESERVE);
    assert_eq!(standard.total_reserve(), 4_608);
    assert_eq!(standard.usable_input, 27_392);

    let long = InputBudget::new(128_001, ContextMode::Long, 8_192);
    assert_eq!(long.effective_window, LONG_CONTEXT_CAP);
    assert_eq!(long.output_reserve, 8_192);
    assert_eq!(long.usable_input, 119_296);

    let model_bound = InputBudget::new(31_999, ContextMode::Standard, 4_097);
    assert_eq!(model_bound.effective_window, 31_999);
    assert_eq!(model_bound.output_reserve, 4_097);
    assert_eq!(model_bound.usable_input, 27_390);
}

#[test]
fn small_local_context_leaves_room_for_a_question_and_bounded_answer() {
    let budget = InputBudget::new(4096, ContextMode::Standard, 1024);
    assert_eq!(budget.output_reserve, 1024);
    assert_eq!(budget.usable_input, 2560);
    assert!(budget.require_fits(2560).is_ok());
    assert!(budget.require_fits(2561).is_err());
}

#[test]
fn budget_boundaries_saturate_and_exact_fit_is_accepted() {
    let no_input = InputBudget::new(4_607, ContextMode::Long, u32::MAX);
    assert_eq!(no_input.usable_input, 0);
    assert_eq!(no_input.total_reserve(), u32::MAX);

    let exact = InputBudget::new(10_000, ContextMode::Long, 4_096);
    assert_eq!(exact.usable_input, 5_392);
    assert_eq!(exact.require_fits(5_392).unwrap(), 5_392);
    let error = exact.require_fits(5_393).unwrap_err();
    assert_eq!(error.code, AppErrorCode::ContextTooLarge);
}

#[test]
fn unicode_scalars_and_every_rendered_delimiter_are_counted_conservatively() {
    let payload = "ASCII 中文 😀 e\u{301} $x^2$ <context>\u{202e}";
    assert_eq!(
        conservative_token_count(payload),
        payload.chars().count() as u64
    );
    assert!(payload.len() > payload.chars().count());

    let parts = [
        "<segment source=\"local_text\">",
        payload,
        "</segment>",
        "user:",
    ];
    let expected = parts
        .iter()
        .map(|part| part.chars().count() as u64)
        .sum::<u64>()
        + 16;
    assert_eq!(conservative_rendered_count(parts, 16), expected);
}
