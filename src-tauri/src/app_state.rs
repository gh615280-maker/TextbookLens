use std::{path::PathBuf, sync::Arc};

use tauri::{AppHandle, Manager, Runtime};
use tracing_appender::non_blocking::WorkerGuard;

use crate::{
    ai::registry::ProviderCapabilityRegistry,
    credentials::CredentialStore,
    db::Database,
    documents::import::ImportCancellationRegistry,
    errors::AppResult,
    indexing::{coordinator::IndexOperationRegistry, state::IndexCancellationRegistry},
    learning::{
        book_preparation::BookPreparationRegistry, preparation::PreparationRegistry,
        registry::LearningRequestRegistry,
    },
    maintenance::{gate::MaintenanceGate, storage::prepare_app_data_paths},
};

#[derive(Clone, Debug)]
pub struct AppPaths {
    pub root: PathBuf,
    pub books: PathBuf,
    pub cache: PathBuf,
    pub logs: PathBuf,
    pub database: PathBuf,
}

impl AppPaths {
    pub fn from_app<R: Runtime>(app: &AppHandle<R>) -> AppResult<Self> {
        let root = app
            .path()
            .app_data_dir()
            .map_err(crate::errors::AppError::local_io)?;
        let prepared = prepare_app_data_paths(&root)
            .map_err(|_| crate::errors::AppError::new(crate::errors::AppErrorCode::LocalIoError))?;
        Ok(Self {
            database: prepared.database,
            root: prepared.root,
            books: prepared.books,
            cache: prepared.cache,
            logs: prepared.logs,
        })
    }

    pub fn indexing_scratch(&self) -> PathBuf {
        self.cache.join("indexing-pages")
    }
}

pub struct AppState {
    pub db: Database,
    pub paths: AppPaths,
    pub maintenance_gate: MaintenanceGate,
    pub import_cancellations: ImportCancellationRegistry,
    pub indexing_cancellations: IndexCancellationRegistry,
    pub indexing_operations: IndexOperationRegistry,
    pub learning_preparations: PreparationRegistry,
    pub book_learning_preparations: BookPreparationRegistry,
    pub learning_requests: LearningRequestRegistry,
    pub log_guard: WorkerGuard,
    pub credential_store: Arc<dyn CredentialStore>,
    pub provider_capabilities: ProviderCapabilityRegistry,
}

impl AppState {
    pub fn new(
        db: Database,
        paths: AppPaths,
        log_guard: WorkerGuard,
        credential_store: Arc<dyn CredentialStore>,
        provider_capabilities: ProviderCapabilityRegistry,
    ) -> Self {
        let maintenance_gate = MaintenanceGate::default();
        Self {
            db,
            paths,
            maintenance_gate: maintenance_gate.clone(),
            import_cancellations: ImportCancellationRegistry::default(),
            indexing_cancellations: IndexCancellationRegistry::default(),
            indexing_operations: IndexOperationRegistry::with_maintenance_gate(
                maintenance_gate.clone(),
            ),
            learning_preparations: PreparationRegistry::default(),
            book_learning_preparations: BookPreparationRegistry::default(),
            learning_requests: LearningRequestRegistry::with_maintenance_gate(maintenance_gate),
            log_guard,
            credential_store,
            provider_capabilities,
        }
    }
}

impl Drop for AppState {
    fn drop(&mut self) {
        self.maintenance_gate.shutdown();
        self.learning_requests.shutdown();
    }
}
