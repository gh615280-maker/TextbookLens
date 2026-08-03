use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::errors::{AppError, AppErrorCode, AppResult};

pub const MAX_TEACHING_INSTRUCTION_CODE_POINTS: usize = 1_000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "teaching.ts")]
pub struct TeachingInstructionDto {
    pub instruction: String,
    pub revision: u64,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "teaching.ts")]
pub struct UpdateTeachingInstruction {
    pub instruction: String,
    pub expected_revision: u64,
}

pub(crate) fn normalize_teaching_instruction(value: &str) -> AppResult<String> {
    let mut normalized = String::with_capacity(value.len());
    let mut code_points = value.chars().peekable();
    while let Some(code_point) = code_points.next() {
        if code_point == '\r' {
            if code_points.peek() == Some(&'\n') {
                code_points.next();
            }
            normalized.push('\n');
        } else {
            normalized.push(code_point);
        }
    }

    validate_teaching_instruction(&normalized)?;
    if normalized.chars().all(char::is_whitespace) {
        Ok(String::new())
    } else {
        Ok(normalized)
    }
}

pub(crate) fn validate_teaching_instruction(value: &str) -> AppResult<()> {
    if value.chars().count() > MAX_TEACHING_INSTRUCTION_CODE_POINTS
        || value
            .chars()
            .any(|code_point| code_point.is_control() && code_point != '\n' && code_point != '\t')
    {
        return Err(AppError::new(AppErrorCode::InvalidInput));
    }
    Ok(())
}
