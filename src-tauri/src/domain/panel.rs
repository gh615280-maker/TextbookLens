use std::fmt;

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

pub const MAX_LEARNING_OUTPUT_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_SAFE_LEARNING_ERROR_CODE_CHARS: usize = 64;
pub const MIN_PANEL_WIDTH_PX: f64 = 320.0;
pub const MIN_PANEL_HEIGHT_PX: f64 = 240.0;
pub const MAX_PANEL_DIMENSION_PX: f64 = 8192.0;
pub const DEFAULT_PANEL_WIDTH_PX: f64 = 400.0;
pub const DEFAULT_PANEL_HEIGHT_PX: f64 = 520.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "panel.ts")]
pub enum LearningRequestStatus {
    Preparing,
    Streaming,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "panel.ts")]
pub struct LearningUsage {
    pub input_tokens: Option<u32>,
    pub output_tokens: Option<u32>,
}

impl LearningUsage {
    pub fn new(
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
    ) -> Result<Self, &'static str> {
        if input_tokens.is_none() && output_tokens.is_none() {
            return Err("learning usage must contain at least one token count");
        }
        Ok(Self {
            input_tokens: input_tokens
                .map(u32::try_from)
                .transpose()
                .map_err(|_| "learning input token count is out of range")?,
            output_tokens: output_tokens
                .map(u32::try_from)
                .transpose()
                .map_err(|_| "learning output token count is out of range")?,
        })
    }

    fn validate(&self) -> Result<(), &'static str> {
        if self.input_tokens.is_none() && self.output_tokens.is_none() {
            return Err("learning usage must contain at least one token count");
        }
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SerializedLearningUsage {
    input_tokens: Option<u32>,
    output_tokens: Option<u32>,
}

impl<'de> Deserialize<'de> for LearningUsage {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = SerializedLearningUsage::deserialize(deserializer)?;
        let usage = Self {
            input_tokens: value.input_tokens,
            output_tokens: value.output_tokens,
        };
        usage.validate().map_err(serde::de::Error::custom)?;
        Ok(usage)
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "panel.ts")]
pub struct SafeLearningError {
    pub code: String,
}

impl SafeLearningError {
    pub fn new(code: impl Into<String>) -> Result<Self, &'static str> {
        let value = Self { code: code.into() };
        value.validate()?;
        Ok(value)
    }

    fn validate(&self) -> Result<(), &'static str> {
        let length = self.code.chars().count();
        if !(1..=MAX_SAFE_LEARNING_ERROR_CODE_CHARS).contains(&length)
            || !self
                .code
                .bytes()
                .all(|value| value.is_ascii_uppercase() || value.is_ascii_digit() || value == b'_')
        {
            return Err("learning error code is invalid");
        }
        Ok(())
    }
}

impl fmt::Debug for SafeLearningError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SafeLearningError")
            .field("code", &self.code)
            .finish()
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SerializedSafeLearningError {
    code: String,
}

impl<'de> Deserialize<'de> for SafeLearningError {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = SerializedSafeLearningError::deserialize(deserializer)?;
        Self::new(value.code).map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "panel.ts")]
pub struct LearningRequestSnapshot {
    pub request_id: Uuid,
    pub conversation_id: Option<Uuid>,
    pub status: LearningRequestStatus,
    pub text: String,
    pub usage: Option<LearningUsage>,
    pub safe_error: Option<SafeLearningError>,
    pub last_seq: u32,
}

impl LearningRequestSnapshot {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        request_id: Uuid,
        conversation_id: Option<Uuid>,
        status: LearningRequestStatus,
        text: String,
        usage: Option<LearningUsage>,
        safe_error: Option<SafeLearningError>,
        last_seq: u32,
    ) -> Result<Self, &'static str> {
        let value = Self {
            request_id,
            conversation_id,
            status,
            text,
            usage,
            safe_error,
            last_seq,
        };
        value.validate()?;
        Ok(value)
    }

    fn validate(&self) -> Result<(), &'static str> {
        if self.text.len() > MAX_LEARNING_OUTPUT_BYTES {
            return Err("learning output exceeds the in-memory byte limit");
        }
        if let Some(usage) = &self.usage {
            usage.validate()?;
        }
        if let Some(error) = &self.safe_error {
            error.validate()?;
        }
        match self.status {
            LearningRequestStatus::Completed => {
                if self.conversation_id.is_none()
                    || self.text.trim().is_empty()
                    || self.safe_error.is_some()
                {
                    return Err("completed learning snapshot is inconsistent");
                }
            }
            LearningRequestStatus::Failed => {
                if self.conversation_id.is_some() || self.safe_error.is_none() {
                    return Err("failed learning snapshot is inconsistent");
                }
            }
            LearningRequestStatus::Preparing
            | LearningRequestStatus::Streaming
            | LearningRequestStatus::Cancelled => {
                if self.conversation_id.is_some() || self.safe_error.is_some() {
                    return Err("non-completed learning snapshot is inconsistent");
                }
            }
        }
        Ok(())
    }
}

impl fmt::Debug for LearningRequestSnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LearningRequestSnapshot")
            .field("request_id", &"<redacted>")
            .field(
                "conversation_id",
                &self.conversation_id.map(|_| "<redacted>"),
            )
            .field("status", &self.status)
            .field("text_bytes", &self.text.len())
            .field("usage", &self.usage)
            .field("safe_error", &self.safe_error)
            .field("last_seq", &self.last_seq)
            .finish()
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SerializedLearningRequestSnapshot {
    request_id: Uuid,
    conversation_id: Option<Uuid>,
    status: LearningRequestStatus,
    text: String,
    usage: Option<LearningUsage>,
    safe_error: Option<SafeLearningError>,
    last_seq: u32,
}

impl<'de> Deserialize<'de> for LearningRequestSnapshot {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = SerializedLearningRequestSnapshot::deserialize(deserializer)?;
        Self::new(
            value.request_id,
            value.conversation_id,
            value.status,
            value.text,
            value.usage,
            value.safe_error,
            value.last_seq,
        )
        .map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, PartialEq, Serialize, TS)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
#[ts(export_to = "panel.ts")]
pub enum LearningRequestEventPayload {
    Preparing,
    TextDelta {
        text: String,
    },
    Usage {
        input_tokens: Option<u32>,
        output_tokens: Option<u32>,
    },
    Completed {
        conversation_id: Uuid,
    },
    Failed {
        safe_error: SafeLearningError,
    },
    Cancelled,
}

impl LearningRequestEventPayload {
    pub fn text_delta(text: String) -> Result<Self, &'static str> {
        let value = Self::TextDelta { text };
        value.validate()?;
        Ok(value)
    }

    pub fn usage(
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
    ) -> Result<Self, &'static str> {
        let usage = LearningUsage::new(input_tokens, output_tokens)?;
        Ok(Self::Usage {
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
        })
    }

    fn validate(&self) -> Result<(), &'static str> {
        match self {
            Self::TextDelta { text } => {
                if text.is_empty() || text.len() > MAX_LEARNING_OUTPUT_BYTES {
                    return Err("learning text delta is empty or exceeds its byte limit");
                }
            }
            Self::Usage {
                input_tokens,
                output_tokens,
            } => LearningUsage {
                input_tokens: *input_tokens,
                output_tokens: *output_tokens,
            }
            .validate()?,
            Self::Failed { safe_error } => safe_error.validate()?,
            Self::Preparing | Self::Completed { .. } | Self::Cancelled => {}
        }
        Ok(())
    }
}

impl fmt::Debug for LearningRequestEventPayload {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Preparing => formatter.write_str("Preparing"),
            Self::TextDelta { text } => formatter
                .debug_struct("TextDelta")
                .field("text_bytes", &text.len())
                .finish(),
            Self::Usage {
                input_tokens,
                output_tokens,
            } => formatter
                .debug_struct("Usage")
                .field("input_tokens", input_tokens)
                .field("output_tokens", output_tokens)
                .finish(),
            Self::Completed { .. } => formatter
                .debug_struct("Completed")
                .field("conversation_id", &"<redacted>")
                .finish(),
            Self::Failed { safe_error } => formatter
                .debug_struct("Failed")
                .field("safe_error", safe_error)
                .finish(),
            Self::Cancelled => formatter.write_str("Cancelled"),
        }
    }
}

#[derive(Deserialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
enum SerializedLearningRequestEventPayload {
    Preparing,
    TextDelta {
        text: String,
    },
    Usage {
        input_tokens: Option<u32>,
        output_tokens: Option<u32>,
    },
    Completed {
        conversation_id: Uuid,
    },
    Failed {
        safe_error: SafeLearningError,
    },
    Cancelled,
}

impl<'de> Deserialize<'de> for LearningRequestEventPayload {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let serialized = SerializedLearningRequestEventPayload::deserialize(deserializer)?;
        let value = match serialized {
            SerializedLearningRequestEventPayload::Preparing => Self::Preparing,
            SerializedLearningRequestEventPayload::TextDelta { text } => Self::TextDelta { text },
            SerializedLearningRequestEventPayload::Usage {
                input_tokens,
                output_tokens,
            } => Self::Usage {
                input_tokens,
                output_tokens,
            },
            SerializedLearningRequestEventPayload::Completed { conversation_id } => {
                Self::Completed { conversation_id }
            }
            SerializedLearningRequestEventPayload::Failed { safe_error } => {
                Self::Failed { safe_error }
            }
            SerializedLearningRequestEventPayload::Cancelled => Self::Cancelled,
        };
        value.validate().map_err(serde::de::Error::custom)?;
        Ok(value)
    }
}

#[derive(Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "panel.ts")]
pub struct LearningRequestEvent {
    pub request_id: Uuid,
    pub seq: u32,
    pub event: LearningRequestEventPayload,
}

impl LearningRequestEvent {
    pub fn new(
        request_id: Uuid,
        seq: u32,
        event: LearningRequestEventPayload,
    ) -> Result<Self, &'static str> {
        if seq == 0 {
            return Err("learning event sequence must be positive");
        }
        event.validate()?;
        Ok(Self {
            request_id,
            seq,
            event,
        })
    }
}

impl fmt::Debug for LearningRequestEvent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LearningRequestEvent")
            .field("request_id", &"<redacted>")
            .field("seq", &self.seq)
            .field("event", &self.event)
            .finish()
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SerializedLearningRequestEvent {
    request_id: Uuid,
    seq: u32,
    event: LearningRequestEventPayload,
}

impl<'de> Deserialize<'de> for LearningRequestEvent {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = SerializedLearningRequestEvent::deserialize(deserializer)?;
        Self::new(value.request_id, value.seq, value.event).map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Copy, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "panel.ts")]
pub struct PanelGeometry {
    pub x_ratio: f64,
    pub y_ratio: f64,
    pub width_px: f64,
    pub height_px: f64,
}

impl PanelGeometry {
    pub fn new(
        x_ratio: f64,
        y_ratio: f64,
        width_px: f64,
        height_px: f64,
    ) -> Result<Self, &'static str> {
        let values = [x_ratio, y_ratio, width_px, height_px];
        if values.iter().any(|value| !value.is_finite())
            || !(0.0..=1.0).contains(&x_ratio)
            || !(0.0..=1.0).contains(&y_ratio)
            || !(MIN_PANEL_WIDTH_PX..=MAX_PANEL_DIMENSION_PX).contains(&width_px)
            || !(MIN_PANEL_HEIGHT_PX..=MAX_PANEL_DIMENSION_PX).contains(&height_px)
        {
            return Err("panel geometry is non-finite or out of range");
        }
        Ok(Self {
            x_ratio,
            y_ratio,
            width_px,
            height_px,
        })
    }
}

impl Default for PanelGeometry {
    fn default() -> Self {
        Self {
            x_ratio: 0.5,
            y_ratio: 0.5,
            width_px: DEFAULT_PANEL_WIDTH_PX,
            height_px: DEFAULT_PANEL_HEIGHT_PX,
        }
    }
}

impl fmt::Debug for PanelGeometry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PanelGeometry")
            .field("x_ratio", &self.x_ratio)
            .field("y_ratio", &self.y_ratio)
            .field("width_px", &self.width_px)
            .field("height_px", &self.height_px)
            .finish()
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SerializedPanelGeometry {
    x_ratio: f64,
    y_ratio: f64,
    width_px: f64,
    height_px: f64,
}

impl<'de> Deserialize<'de> for PanelGeometry {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = SerializedPanelGeometry::deserialize(deserializer)?;
        Self::new(
            value.x_ratio,
            value.y_ratio,
            value.width_px,
            value.height_px,
        )
        .map_err(serde::de::Error::custom)
    }
}
