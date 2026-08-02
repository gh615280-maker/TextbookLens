#![allow(linker_messages)]

use std::sync::Arc;
use tauri::Manager;

pub mod app_state;
#[path = "db/books.rs"]
mod book_repository;
pub mod commands;
pub mod credentials;
pub mod db;
pub mod documents;
pub mod domain;
pub mod errors;
pub mod logging;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let paths = app_state::AppPaths::from_app(app.handle())?;
            let log_guard = logging::init(&paths.logs)?;
            let database = db::Database::open(&paths.database)?;
            db::settings::recover_interrupted_imports(database.pool())?;
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
            commands::credentials::list_provider_profiles,
            commands::credentials::delete_provider_profile,
            commands::books::list_books,
            commands::books::get_book,
            commands::books::delete_failed_import,
            commands::documents::begin_import,
            commands::documents::read_book_source,
            commands::documents::begin_parse,
            commands::documents::append_parsed_sections,
            commands::documents::finalize_import,
            commands::documents::cancel_import,
            commands::documents::mark_import_failed,
            commands::documents::retry_import
        ])
        .run(tauri::generate_context!())
        .expect("failed to run TextbookLens");
}
