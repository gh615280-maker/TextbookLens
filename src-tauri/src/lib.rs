#![allow(linker_messages)]

use std::sync::Arc;
use tauri::Manager;

pub mod app_state;
#[allow(dead_code)]
#[path = "db/books.rs"]
mod book_repository;
pub mod commands;
pub mod credentials;
pub mod db;
#[path = "db/documents.rs"]
pub mod document_repository;
pub mod documents;
pub mod domain;
pub mod errors;
pub mod logging;
pub mod retrieval;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let paths = app_state::AppPaths::from_app(app.handle())?;
            let log_guard = logging::init(&paths.logs)?;
            let database = db::Database::open(&paths.database)?;
            db::settings::recover_interrupted_imports(database.pool(), &paths)?;
            let credential_store = Arc::new(credentials::KeyringCredentialStore::new());
            app.manage(app_state::AppState::new(
                database,
                paths,
                log_guard,
                credential_store,
            ));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::annotations::list_annotation_markers,
            commands::credentials::list_provider_profiles,
            commands::credentials::delete_provider_profile,
            commands::books::list_books,
            commands::books::get_book,
            commands::books::delete_failed_import,
            commands::books::list_reader_sections,
            commands::documents::begin_import,
            commands::documents::read_book_source,
            commands::documents::read_derived_text,
            commands::documents::begin_parse,
            commands::documents::append_parsed_sections,
            commands::documents::write_derived_text,
            commands::documents::finalize_import,
            commands::documents::cancel_import,
            commands::documents::mark_import_failed,
            commands::documents::retry_import,
            commands::documents::search_book,
            commands::settings::get_reader_bootstrap,
            commands::settings::save_reading_progress,
            commands::settings::get_reader_settings,
            commands::settings::update_reader_settings
        ])
        .run(tauri::generate_context!())
        .expect("failed to run TextbookLens");
}
