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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "settings.ts")]
pub enum UiLanguage {
    #[serde(rename = "zh-CN")]
    #[ts(rename = "zh-CN")]
    ZhCn,
    #[serde(rename = "zh-TW")]
    #[ts(rename = "zh-TW")]
    ZhTw,
    #[serde(rename = "en")]
    #[ts(rename = "en")]
    En,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "settings.ts")]
pub struct AppSettingsDto {
    pub onboarding_completed: bool,
    pub active_provider_profile_id: Option<Uuid>,
    #[serde(default)]
    #[ts(optional = nullable)]
    pub default_learning_profile_id: Option<Uuid>,
    #[serde(default)]
    #[ts(optional = nullable)]
    pub default_vision_profile_id: Option<Uuid>,
    pub theme: Theme,
    pub context_mode: ContextMode,
    pub ui_language: UiLanguage,
    pub ui_language_initialized: bool,
    pub first_reader_hint_completed: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "settings.ts")]
pub struct ReaderSettingsDto {
    pub font_scale: f64,
    pub line_height: f64,
    pub reader_width: f64,
    pub pdf_zoom: f64,
    pub theme: Theme,
}
