pub mod annotations;
pub mod conversations;
pub mod corrections;
pub mod indexing;
pub(crate) mod local_capabilities;
pub mod messages;
pub mod notes;
pub mod overview;
pub mod providers;
pub mod settings;
pub mod teaching;

use std::{path::Path, time::Duration};

use sqlx::{
    AssertSqlSafe, Executor, Row, SqlSafeStr, SqlStr, SqliteConnection, SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
};

use crate::errors::{AppError, AppResult};

pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!();

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
                .map_err(|_| AppError::new(crate::errors::AppErrorCode::DatabaseError))?;
            let migration_result = async {
                let mut connection = pool.acquire().await.map_err(|_| ())?;
                migrate_fail_closed(&mut connection).await
            }
            .await;
            if migration_result.is_err() {
                pool.close().await;
                return Err(AppError::new(crate::errors::AppErrorCode::DatabaseError));
            }
            Ok(Self { pool })
        })
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }
}

type MigrationResult<T> = Result<T, ()>;

const MIGRATION_TABLE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS _sqlx_migrations (
    version BIGINT PRIMARY KEY,
    description TEXT NOT NULL,
    installed_on TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    success BOOLEAN NOT NULL,
    checksum BLOB NOT NULL,
    execution_time BIGINT NOT NULL
);
"#;

async fn migrate_fail_closed(connection: &mut SqliteConnection) -> MigrationResult<()> {
    let user_version: i64 = sqlx::query_scalar("PRAGMA user_version")
        .fetch_one(&mut *connection)
        .await
        .map_err(|_| ())?;
    if user_version != 0 {
        return Err(());
    }

    let has_history = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM sqlite_schema
         WHERE type = 'table' AND name = '_sqlx_migrations'",
    )
    .fetch_one(&mut *connection)
    .await
    .map_err(|_| ())?
        == 1;
    let applied = if has_history {
        validate_migration_history(connection).await?
    } else {
        let existing_objects: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sqlite_schema
             WHERE name NOT GLOB 'sqlite_*'",
        )
        .fetch_one(&mut *connection)
        .await
        .map_err(|_| ())?;
        if existing_objects != 0 {
            return Err(());
        }
        0
    };

    require_database_integrity(connection).await?;
    for migration in MIGRATOR.migrations.iter().skip(applied) {
        apply_migration_atomically(connection, migration).await?;
    }
    if validate_migration_history(connection).await? != MIGRATOR.migrations.len() {
        return Err(());
    }
    require_database_integrity(connection).await
}

async fn validate_migration_history(connection: &mut SqliteConnection) -> MigrationResult<usize> {
    let columns = sqlx::query(
        "SELECT cid, name, type, \"notnull\" AS is_not_null, dflt_value, pk
         FROM pragma_table_info('_sqlx_migrations')
         ORDER BY cid",
    )
    .fetch_all(&mut *connection)
    .await
    .map_err(|_| ())?;
    let expected_columns = [
        (0_i64, "version", "BIGINT", 0_i64, None, 1_i64),
        (1, "description", "TEXT", 1, None, 0),
        (
            2,
            "installed_on",
            "TIMESTAMP",
            1,
            Some("CURRENT_TIMESTAMP"),
            0,
        ),
        (3, "success", "BOOLEAN", 1, None, 0),
        (4, "checksum", "BLOB", 1, None, 0),
        (5, "execution_time", "BIGINT", 1, None, 0),
    ];
    if columns.len() != expected_columns.len() {
        return Err(());
    }
    for (row, expected) in columns.iter().zip(expected_columns) {
        let actual = (
            row.try_get::<i64, _>("cid").map_err(|_| ())?,
            row.try_get::<String, _>("name").map_err(|_| ())?,
            row.try_get::<String, _>("type").map_err(|_| ())?,
            row.try_get::<i64, _>("is_not_null").map_err(|_| ())?,
            row.try_get::<Option<String>, _>("dflt_value")
                .map_err(|_| ())?,
            row.try_get::<i64, _>("pk").map_err(|_| ())?,
        );
        if actual.0 != expected.0
            || actual.1 != expected.1
            || actual.2 != expected.2
            || actual.3 != expected.3
            || actual.4.as_deref() != expected.4
            || actual.5 != expected.5
        {
            return Err(());
        }
    }

    let rows = sqlx::query(
        "SELECT version, description, success, checksum
         FROM _sqlx_migrations
         ORDER BY version",
    )
    .fetch_all(&mut *connection)
    .await
    .map_err(|_| ())?;
    if rows.len() > MIGRATOR.migrations.len() {
        return Err(());
    }
    for (row, migration) in rows.iter().zip(MIGRATOR.migrations.iter()) {
        let version = row.try_get::<i64, _>("version").map_err(|_| ())?;
        let description = row.try_get::<String, _>("description").map_err(|_| ())?;
        let success = row.try_get::<i64, _>("success").map_err(|_| ())?;
        let checksum = row.try_get::<Vec<u8>, _>("checksum").map_err(|_| ())?;
        if version != migration.version
            || description != migration.description.as_ref()
            || success != 1
            || checksum.as_slice() != migration.checksum.as_ref()
        {
            return Err(());
        }
    }
    Ok(rows.len())
}

async fn apply_migration_atomically(
    connection: &mut SqliteConnection,
    migration: &sqlx::migrate::Migration,
) -> MigrationResult<()> {
    if migration.no_tx {
        sqlx::query("PRAGMA foreign_keys = OFF")
            .execute(&mut *connection)
            .await
            .map_err(|_| ())?;
        let disabled: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
            .fetch_one(&mut *connection)
            .await
            .map_err(|_| ())?;
        if disabled != 0 {
            return Err(());
        }
    }

    let result = apply_migration_transaction(connection, migration).await;
    if result.is_err() {
        let _ = sqlx::query("ROLLBACK").execute(&mut *connection).await;
    }
    if migration.no_tx {
        let enabled = async {
            sqlx::query("PRAGMA foreign_keys = ON")
                .execute(&mut *connection)
                .await
                .map_err(|_| ())?;
            let enabled: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
                .fetch_one(&mut *connection)
                .await
                .map_err(|_| ())?;
            (enabled == 1).then_some(()).ok_or(())
        }
        .await;
        if enabled.is_err() {
            return Err(());
        }
    }
    result
}

async fn apply_migration_transaction(
    connection: &mut SqliteConnection,
    migration: &sqlx::migrate::Migration,
) -> MigrationResult<()> {
    sqlx::query("BEGIN IMMEDIATE")
        .execute(&mut *connection)
        .await
        .map_err(|_| ())?;
    connection
        .execute(MIGRATION_TABLE_SQL)
        .await
        .map_err(|_| ())?;
    let sql = atomic_migration_sql(migration)?;
    connection.execute(sql).await.map_err(|_| ())?;
    if sqlx::query("PRAGMA foreign_key_check")
        .fetch_optional(&mut *connection)
        .await
        .map_err(|_| ())?
        .is_some()
    {
        return Err(());
    }
    sqlx::query(
        "INSERT INTO _sqlx_migrations (
           version, description, success, checksum, execution_time
         ) VALUES (?, ?, 1, ?, 0)",
    )
    .bind(migration.version)
    .bind(migration.description.as_ref())
    .bind(migration.checksum.as_ref())
    .execute(&mut *connection)
    .await
    .map_err(|_| ())?;
    sqlx::query("COMMIT")
        .execute(&mut *connection)
        .await
        .map_err(|_| ())?;
    Ok(())
}

fn atomic_migration_sql(migration: &sqlx::migrate::Migration) -> MigrationResult<SqlStr> {
    if !migration.no_tx {
        return Ok(migration.sql.clone());
    }
    if !matches!(migration.version, 2 | 11 | 16) {
        return Err(());
    }

    let mut body = String::new();
    let mut markers = (false, false, false, false);
    for line in migration.sql.as_str().lines() {
        match line.trim() {
            "-- no-transaction" => {}
            "PRAGMA foreign_keys = OFF;" => markers.0 = true,
            "PRAGMA foreign_keys = ON;" => markers.1 = true,
            "BEGIN IMMEDIATE;" => markers.2 = true,
            "COMMIT;" => markers.3 = true,
            _ => {
                body.push_str(line);
                body.push('\n');
            }
        }
    }
    let expected = if migration.version == 2 {
        (true, true, false, false)
    } else {
        (true, true, true, true)
    };
    if markers != expected || body.trim().is_empty() {
        return Err(());
    }
    Ok(AssertSqlSafe(body).into_sql_str())
}

async fn require_database_integrity(connection: &mut SqliteConnection) -> MigrationResult<()> {
    let integrity = sqlx::query_scalar::<_, String>("PRAGMA integrity_check")
        .fetch_all(&mut *connection)
        .await
        .map_err(|_| ())?;
    if integrity.as_slice() != ["ok"] {
        return Err(());
    }
    if sqlx::query("PRAGMA foreign_key_check")
        .fetch_optional(&mut *connection)
        .await
        .map_err(|_| ())?
        .is_some()
    {
        return Err(());
    }
    Ok(())
}
