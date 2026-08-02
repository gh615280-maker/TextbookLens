use std::{collections::HashSet, fs, path::PathBuf};

use parking_lot::Mutex;
use tauri::{AppHandle, Manager, Runtime};
use uuid::Uuid;

use crate::{db::Database, errors::AppResult};

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
    pub import_cancellations: Mutex<HashSet<Uuid>>,
    pub learning_cancellations: Mutex<HashSet<Uuid>>,
}

impl AppState {
    pub fn new(db: Database, paths: AppPaths) -> Self {
        Self {
            db,
            paths,
            import_cancellations: Mutex::new(HashSet::new()),
            learning_cancellations: Mutex::new(HashSet::new()),
        }
    }
}
