use tauri::State;

use crate::{
    app_state::AppState,
    db::teaching,
    domain::{TeachingInstructionDto, UpdateTeachingInstruction},
    errors::{AppErrorDto, AppResult},
};

#[tauri::command]
pub async fn get_teaching_instruction(
    state: State<'_, AppState>,
) -> Result<TeachingInstructionDto, AppErrorDto> {
    teaching::get_teaching_instruction(state.db.pool())
        .await
        .map_err(AppErrorDto::from)
}

#[tauri::command]
pub async fn update_teaching_instruction(
    state: State<'_, AppState>,
    update: UpdateTeachingInstruction,
) -> Result<TeachingInstructionDto, AppErrorDto> {
    persist_teaching_instruction(state.db.pool(), update)
        .await
        .map_err(AppErrorDto::from)
}

async fn persist_teaching_instruction(
    pool: &sqlx::SqlitePool,
    update: UpdateTeachingInstruction,
) -> AppResult<TeachingInstructionDto> {
    teaching::update_teaching_instruction(pool, update).await
}
