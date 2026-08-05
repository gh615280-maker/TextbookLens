use tauri::State;
use uuid::Uuid;

use crate::{
    app_state::AppState,
    db::{
        annotations::{self, AnnotationMarkerDto},
        notes::{self, CreateNote, DeleteNote, NoteDto, UpdateNote},
    },
    domain::ContentAnchor,
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

#[tauri::command]
pub async fn create_note(
    state: State<'_, AppState>,
    book_id: Uuid,
    section_id: Uuid,
    anchor: ContentAnchor,
    selected_text: Option<String>,
    note_text: String,
) -> Result<NoteDto, AppErrorDto> {
    notes::create_note(
        state.db.pool(),
        CreateNote {
            book_id,
            section_id,
            anchor,
            selected_text,
            note_text,
        },
    )
    .await
    .map_err(AppErrorDto::from)
}

#[tauri::command]
pub async fn update_note(
    state: State<'_, AppState>,
    book_id: Uuid,
    note_id: Uuid,
    expected_revision: u32,
    note_text: String,
) -> Result<NoteDto, AppErrorDto> {
    notes::update_note(
        state.db.pool(),
        UpdateNote {
            book_id,
            note_id,
            expected_revision,
            note_text,
        },
    )
    .await
    .map_err(AppErrorDto::from)
}

#[tauri::command]
pub async fn delete_note(
    state: State<'_, AppState>,
    book_id: Uuid,
    note_id: Uuid,
    expected_revision: u32,
) -> Result<(), AppErrorDto> {
    notes::delete_note(
        state.db.pool(),
        DeleteNote {
            book_id,
            note_id,
            expected_revision,
        },
    )
    .await
    .map_err(AppErrorDto::from)
}

#[tauri::command]
pub async fn get_note(
    state: State<'_, AppState>,
    book_id: Uuid,
    note_id: Uuid,
) -> Result<NoteDto, AppErrorDto> {
    notes::get_note(state.db.pool(), book_id, note_id)
        .await
        .map_err(AppErrorDto::from)
}

#[tauri::command]
pub async fn list_notes(
    state: State<'_, AppState>,
    book_id: Uuid,
) -> Result<Vec<NoteDto>, AppErrorDto> {
    notes::list_notes(state.db.pool(), book_id)
        .await
        .map_err(AppErrorDto::from)
}
