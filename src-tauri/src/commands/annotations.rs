use tauri::State;
use uuid::Uuid;

use crate::{
    app_state::AppState,
    db::annotations::{self, AnnotationMarkerDto},
    errors::AppErrorDto,
};

#[tauri::command]
pub async fn list_annotation_markers(
    state: State<'_, AppState>,
    book_id: Uuid,
) -> Result<Vec<AnnotationMarkerDto>, AppErrorDto> {
    annotations::list_annotation_markers(state.db.pool(), book_id)
        .await
        .map_err(AppErrorDto::from)
}
