#![allow(linker_messages)]

use tauri::Manager;

pub mod app_state;
pub mod db;
pub mod domain;
pub mod errors;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let paths = app_state::AppPaths::from_app(app.handle())?;
            let database = db::Database::open(&paths.database)?;
            db::settings::recover_interrupted_imports(database.pool())?;
            app.manage(app_state::AppState::new(database, paths));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![])
        .run(tauri::generate_context!())
        .expect("failed to run TextbookLens");
}
