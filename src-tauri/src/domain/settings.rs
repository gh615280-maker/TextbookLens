use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "settings.ts")]
pub enum Theme {
    Light,
    Dark,
    System,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "settings.ts")]
pub enum ContextMode {
    Standard,
    Long,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "settings.ts")]
pub struct AppSettingsDto {
    pub onboarding_completed: bool,
    pub active_provider_profile_id: Option<Uuid>,
    pub theme: Theme,
    pub context_mode: ContextMode,
    pub ui_language: String,
}
