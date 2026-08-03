use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::BookSummary;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "onboarding.ts")]
pub enum OnboardingStep {
    Book,
    Provider,
    Ready,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "onboarding.ts")]
pub struct OnboardingStateDto {
    pub step: OnboardingStep,
    pub selected_book: Option<BookSummary>,
    pub has_ready_book: bool,
    pub learning_profile_connected: bool,
    pub vision_profile_connected: bool,
    pub local_text_quality: LocalTextQuality,
    pub can_skip_onboarding: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "onboarding.ts")]
pub enum LocalTextQuality {
    Ready,
    Pending,
    Unavailable,
}

#[cfg(test)]
mod tests {
    use super::{LocalTextQuality, OnboardingStep};

    #[test]
    fn onboarding_contract_uses_only_derived_steps_and_local_quality_labels() {
        assert_eq!(
            serde_json::to_string(&OnboardingStep::Provider).unwrap(),
            "\"provider\""
        );
        assert_eq!(
            serde_json::to_string(&LocalTextQuality::Pending).unwrap(),
            "\"pending\""
        );
    }
}
