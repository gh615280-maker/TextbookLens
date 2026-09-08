use crate::{
    domain::ContextMode,
    errors::{AppError, AppErrorCode, AppResult},
};

pub const STANDARD_CONTEXT_CAP: u32 = 32_000;
pub const LONG_CONTEXT_CAP: u32 = 128_000;
pub const MINIMUM_OUTPUT_RESERVE: u32 = 4_096;
pub const PROTOCOL_RESERVE: u32 = 512;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InputBudget {
    pub effective_window: u32,
    pub output_reserve: u32,
    pub protocol_reserve: u32,
    pub usable_input: u32,
}

impl InputBudget {
    pub fn new(
        model_context_tokens: u32,
        context_mode: ContextMode,
        requested_output_tokens: u32,
    ) -> Self {
        let mode_cap = match context_mode {
            ContextMode::Standard => STANDARD_CONTEXT_CAP,
            ContextMode::Long => LONG_CONTEXT_CAP,
        };
        let effective_window = model_context_tokens.min(mode_cap);
        // Small local models can explicitly reserve less output; the cloud-sized
        // floor would otherwise leave no room for any question.
        let output_reserve =
            if effective_window < 8_192 && requested_output_tokens <= effective_window / 4 {
                requested_output_tokens
            } else {
                requested_output_tokens.max(MINIMUM_OUTPUT_RESERVE)
            };
        let usable_input = effective_window
            .saturating_sub(output_reserve)
            .saturating_sub(PROTOCOL_RESERVE);
        Self {
            effective_window,
            output_reserve,
            protocol_reserve: PROTOCOL_RESERVE,
            usable_input,
        }
    }

    pub const fn total_reserve(self) -> u32 {
        self.output_reserve.saturating_add(self.protocol_reserve)
    }

    pub fn require_fits(self, conservative_tokens: u64) -> AppResult<u32> {
        if conservative_tokens > u64::from(self.usable_input) {
            return Err(AppError::new(AppErrorCode::ContextTooLarge));
        }
        u32::try_from(conservative_tokens).map_err(|_| AppError::new(AppErrorCode::ContextTooLarge))
    }
}

pub fn conservative_token_count(value: &str) -> u64 {
    u64::try_from(value.chars().count()).unwrap_or(u64::MAX)
}

pub fn conservative_rendered_count<'a>(
    rendered_parts: impl IntoIterator<Item = &'a str>,
    framing_tokens: u64,
) -> u64 {
    rendered_parts
        .into_iter()
        .fold(framing_tokens, |total, part| {
            total.saturating_add(conservative_token_count(part))
        })
}
