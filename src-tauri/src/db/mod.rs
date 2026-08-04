pub mod annotations;
pub mod corrections;
pub mod indexing;
pub mod providers;
pub mod settings;
pub mod teaching;

use std::{path::Path, time::Duration};

use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
};

use crate::errors::{AppError, AppResult};

pub struct Database {
    pool: SqlitePool,
}

impl Database {
    pub fn open(path: impl AsRef<Path>) -> AppResult<Self> {
        let options = SqliteConnectOptions::new()
            .filename(path.as_ref())
            .create_if_missing(true)
            .foreign_keys(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .busy_timeout(Duration::from_secs(5));

        tauri::async_runtime::block_on(async move {
            let pool = SqlitePoolOptions::new()
                .max_connections(5)
                .connect_with(options)
                .await
                .map_err(AppError::from)?;
            sqlx::migrate!()
                .run(&pool)
                .await
                .map_err(AppError::database)?;
            Ok(Self { pool })
        })
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }
}
