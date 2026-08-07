#![allow(linker_messages)]

use std::sync::Arc;
use tauri::Manager;

pub mod ai;
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
pub mod extraction;
pub mod indexing;
pub mod learning;
pub mod logging;
pub mod maintenance;
pub mod retrieval;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let provider_capabilities = ai::registry::ProviderCapabilityRegistry::load_embedded()
        .expect("provider capability registry must be valid before opening a window");
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let raw_root = app.path().app_data_dir()?;
            let credential_store: Arc<dyn credentials::CredentialStore> =
                Arc::new(credentials::KeyringCredentialStore::new());
            match std::fs::symlink_metadata(&raw_root) {
                Ok(_) => {
                    let clear_outcome = tauri::async_runtime::block_on(
                        maintenance::clear_all::recover_pending_clear(
                            &raw_root,
                            credential_store.as_ref(),
                        ),
                    )?;
                    if clear_outcome
                        == maintenance::clear_all::ClearStartupOutcome::CredentialCleanupRequired
                    {
                        return Err(
                            errors::AppError::new(errors::AppErrorCode::RequestConflict).into()
                        );
                    }
                    maintenance::restore::recover_pending_restore(&raw_root)?;
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
            let paths = app_state::AppPaths::from_app(app.handle())?;
            let log_guard = logging::init(&paths.logs)?;
            let database = db::Database::open(&paths.database)?;
            maintenance::delete_book::recover_pending_deletions(database.pool(), &paths)?;
            db::settings::recover_interrupted_imports(database.pool(), &paths)?;
            indexing::recovery::recover_on_startup(database.pool(), &paths.indexing_scratch())?;
            tauri::async_runtime::block_on(extraction::service::recover_on_startup(
                database.pool(),
            ))?;
            tauri::async_runtime::block_on(
                indexing::remote_cleanup::recover_remote_cleanup_on_startup(
                    database.pool(),
                    credential_store.as_ref(),
                ),
            )?;
            let app_state = app_state::AppState::new(
                database,
                paths,
                log_guard,
                credential_store,
                provider_capabilities,
            );
            app.manage(app_state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::annotations::list_annotation_markers,
            commands::annotations::create_note,
            commands::annotations::update_note,
            commands::annotations::delete_note,
            commands::annotations::get_note,
            commands::annotations::list_notes,
            commands::conversations::get_learning_conversation,
            commands::conversations::delete_learning_conversation,
            commands::conversations::get_book_learning_conversation,
            commands::conversations::list_book_learning_conversation_summaries,
            commands::conversations::delete_book_learning_conversation,
            commands::credentials::validate_and_save_provider_profile,
            commands::credentials::replace_provider_profile_credential,
            commands::providers::list_provider_capabilities,
            commands::providers::list_provider_profiles,
            commands::providers::set_active_provider_profile,
            commands::providers::set_default_provider_profile,
            commands::providers::update_provider_operation_consent,
            commands::providers::reset_provider_operation_consents,
            commands::providers::delete_provider_profile,
            commands::onboarding::get_onboarding_state,
            commands::overview::get_learning_overview,
            commands::books::list_books,
            commands::books::get_book,
            commands::books::delete_book,
            commands::books::delete_failed_import,
            commands::books::list_reader_sections,
            commands::books::ensure_pdf_page_sections,
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
            commands::extraction::prepare_book_extraction,
            commands::extraction::cancel_book_extraction,
            commands::extraction::get_book_extraction,
            commands::indexing::confirm_index_operation,
            commands::indexing::create_index_run,
            commands::indexing::authorize_index_run,
            commands::indexing::claim_index_render_batch,
            commands::indexing::read_claimed_index_source,
            commands::indexing::submit_index_render_batch,
            commands::indexing::report_index_render_failure,
            commands::indexing::pause_index_run,
            commands::indexing::resume_index_run,
            commands::indexing::cancel_index_run,
            commands::indexing::retry_index_page,
            commands::indexing::get_index_run_aggregate,
            commands::indexing::find_current_index_run_for_book,
            commands::indexing::list_index_page_reviews,
            commands::indexing::get_index_page_review,
            commands::indexing::list_index_page_corrections,
            commands::indexing::save_index_correction,
            commands::indexing::resolve_index_correction_conflict,
            commands::indexing::delete_index_correction,
            commands::learning::prepare_learning_request,
            commands::learning::prepare_book_learning_request,
            commands::learning::authorize_book_learning_request,
            commands::learning::discard_book_learning_preparation,
            commands::learning::authorize_learning_request,
            commands::learning::stage_region_capture,
            commands::learning::discard_learning_preparation,
            commands::learning::invalidate_learning_preparations,
            commands::learning::start_learning_request,
            commands::learning::start_book_learning_request,
            commands::learning::subscribe_learning_request,
            commands::learning::get_learning_request_snapshot,
            commands::learning::cancel_learning_request,
            commands::learning::start_conversation_followup,
            commands::maintenance::get_maintenance_status,
            commands::maintenance::get_storage_usage,
            commands::maintenance::open_app_data_directory,
            commands::maintenance::create_local_backup,
            commands::maintenance::restore_local_backup,
            commands::maintenance::clear_all_textbooklens_data,
            commands::maintenance::restart_application,
            commands::settings::get_app_settings,
            commands::settings::initialize_ui_language,
            commands::settings::update_ui_language,
            commands::settings::complete_first_reader_hint,
            commands::settings::get_reader_bootstrap,
            commands::settings::save_reading_progress,
            commands::settings::get_reader_settings,
            commands::settings::update_reader_settings,
            commands::teaching::get_teaching_instruction,
            commands::teaching::update_teaching_instruction,
            commands::teaching::start_teaching_test,
            commands::teaching::cancel_teaching_test
        ])
        .run(tauri::generate_context!())
        .expect("failed to run TextbookLens");
}
