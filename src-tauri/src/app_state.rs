use std::{collections::HashSet, fs, path::PathBuf, sync::Arc};

use parking_lot::Mutex;
use tauri::{AppHandle, Manager, Runtime};
use tracing_appender::non_blocking::WorkerGuard;
use uuid::Uuid;

use crate::{
    ai::registry::ProviderCapabilityRegistry,
    credentials::CredentialStore,
    db::Database,
    documents::import::ImportCancellationRegistry,
    errors::AppResult,
    indexing::{coordinator::IndexOperationRegistry, state::IndexCancellationRegistry},
    learning::preparation::PreparationRegistry,
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
        let books = root.join("books");
        let cache = root.join("cache");
        let logs = root.join("logs");
        for directory in [&root, &books, &cache, &logs] {
            fs::create_dir_all(directory)?;
        }
        Ok(Self {
            database: root.join("library.sqlite3"),
            root,
            books,
            cache,
            logs,
        })
    }

    pub fn indexing_scratch(&self) -> PathBuf {
        self.cache.join("indexing-pages")
    }
}

pub struct AppState {
    pub db: Database,
    pub paths: AppPaths,
    pub import_cancellations: ImportCancellationRegistry,
    pub indexing_cancellations: IndexCancellationRegistry,
    pub indexing_operations: IndexOperationRegistry,
    pub learning_preparations: PreparationRegistry,
    pub learning_cancellations: Mutex<HashSet<Uuid>>,
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
        Self {
            db,
            paths,
            import_cancellations: ImportCancellationRegistry::default(),
            indexing_cancellations: IndexCancellationRegistry::default(),
            indexing_operations: IndexOperationRegistry::default(),
            learning_preparations: PreparationRegistry::default(),
            learning_cancellations: Mutex::new(HashSet::new()),
            log_guard,
            credential_store,
            provider_capabilities,
        }
    }
}
