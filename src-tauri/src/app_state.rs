use std::{
    collections::{HashMap, HashSet},
    fs,
    path::PathBuf,
    sync::Arc,
};

use parking_lot::Mutex;
use tauri::{AppHandle, Manager, Runtime};
use tokio_util::sync::CancellationToken;
use tracing_appender::non_blocking::WorkerGuard;
use uuid::Uuid;

use crate::{credentials::CredentialStore, db::Database, errors::AppResult};

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
}

pub struct AppState {
    pub db: Database,
    pub paths: AppPaths,
    pub import_cancellations: Arc<Mutex<HashMap<Uuid, CancellationToken>>>,
    pub learning_cancellations: Mutex<HashSet<Uuid>>,
    pub log_guard: WorkerGuard,
    pub credential_store: Arc<dyn CredentialStore>,
}

impl AppState {
    pub fn new(
        db: Database,
        paths: AppPaths,
        log_guard: WorkerGuard,
        credential_store: Arc<dyn CredentialStore>,
    ) -> Self {
        Self {
            db,
            paths,
            import_cancellations: Arc::new(Mutex::new(HashMap::new())),
            learning_cancellations: Mutex::new(HashSet::new()),
            log_guard,
            credential_store,
        }
    }
}
