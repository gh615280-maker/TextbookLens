use std::collections::HashSet;

use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::{
    app_state::AppPaths,
    documents::storage::{recover_import_storage, remove_book_directory},
    domain::{AppSettingsDto, ContextMode, DocumentLocator, ReaderSettingsDto, Theme, UiLanguage},
    errors::{AppError, AppErrorCode, AppResult},
};

pub async fn get_app_settings(pool: &SqlitePool) -> AppResult<AppSettingsDto> {
    let row = sqlx::query(
        "SELECT onboarding_completed, active_provider_profile_id, default_learning_profile_id, default_vision_profile_id, theme, context_mode, ui_language, ui_language_initialized, first_reader_hint_completed FROM app_settings WHERE id = 1",
    )
    .fetch_one(pool)
    .await?;
    app_settings_from_row(&row)
}

pub async fn initialize_ui_language(
    pool: &SqlitePool,
    detected: UiLanguage,
) -> AppResult<AppSettingsDto> {
    sqlx::query(
        "UPDATE app_settings SET ui_language = ?, ui_language_initialized = 1 WHERE id = 1 AND ui_language_initialized = 0",
    )
    .bind(ui_language_name(detected))
    .execute(pool)
    .await?;
    get_app_settings(pool).await
}

pub async fn update_ui_language(
    pool: &SqlitePool,
    language: UiLanguage,
) -> AppResult<AppSettingsDto> {
    let result = sqlx::query(
        "UPDATE app_settings SET ui_language = ?, ui_language_initialized = 1 WHERE id = 1",
    )
    .bind(ui_language_name(language))
    .execute(pool)
    .await?;
    if result.rows_affected() != 1 {
        return Err(AppError::new(AppErrorCode::DatabaseError));
    }
    get_app_settings(pool).await
}

pub async fn complete_first_reader_hint(pool: &SqlitePool) -> AppResult<AppSettingsDto> {
    let result =
        sqlx::query("UPDATE app_settings SET first_reader_hint_completed = 1 WHERE id = 1")
            .execute(pool)
            .await?;
    if result.rows_affected() != 1 {
        return Err(AppError::new(AppErrorCode::DatabaseError));
    }
    get_app_settings(pool).await
}

pub async fn get_reader_settings(pool: &SqlitePool) -> AppResult<ReaderSettingsDto> {
    let row = sqlx::query(
        "SELECT font_scale, line_height, reader_width, pdf_zoom, theme FROM app_settings WHERE id = 1",
    )
    .fetch_one(pool)
    .await?;
    Ok(ReaderSettingsDto {
        font_scale: row.try_get("font_scale")?,
        line_height: row.try_get("line_height")?,
        reader_width: row.try_get("reader_width")?,
        pdf_zoom: row.try_get("pdf_zoom")?,
        theme: parse_theme(&row.try_get::<String, _>("theme")?)?,
    })
}

pub async fn update_reader_settings(
    pool: &SqlitePool,
    settings: ReaderSettingsDto,
) -> AppResult<ReaderSettingsDto> {
    if !valid_settings(&settings) {
        return Err(AppError::new(AppErrorCode::InvalidInput));
    }
    sqlx::query(
        "UPDATE app_settings SET font_scale = ?, line_height = ?, reader_width = ?, pdf_zoom = ?, theme = ? WHERE id = 1",
    )
    .bind(settings.font_scale)
    .bind(settings.line_height)
    .bind(settings.reader_width)
    .bind(settings.pdf_zoom)
    .bind(theme_name(&settings.theme))
    .execute(pool)
    .await?;
    Ok(settings)
}

pub async fn save_reading_progress(
    pool: &SqlitePool,
    book_id: Uuid,
    progress: f64,
    locator: &DocumentLocator,
) -> AppResult<()> {
    if !progress.is_finite() || !(0.0..=1.0).contains(&progress) {
        return Err(AppError::new(AppErrorCode::InvalidInput));
    }
    let locator_json = serde_json::to_string(locator).map_err(AppError::database)?;
    let result = sqlx::query(
        "UPDATE books SET reading_progress = ?, last_locator_json = ?, last_opened_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?",
    )
    .bind(progress)
    .bind(locator_json)
    .bind(book_id.to_string())
    .execute(pool)
    .await?;
    if result.rows_affected() != 1 {
        return Err(AppError::new(AppErrorCode::NotFound));
    }
    Ok(())
}

fn valid_settings(settings: &ReaderSettingsDto) -> bool {
    settings.font_scale.is_finite()
        && (0.75..=2.0).contains(&settings.font_scale)
        && settings.line_height.is_finite()
        && (1.2..=2.4).contains(&settings.line_height)
        && settings.reader_width.is_finite()
        && (40.0..=120.0).contains(&settings.reader_width)
        && settings.pdf_zoom.is_finite()
        && (0.5..=3.0).contains(&settings.pdf_zoom)
}

fn app_settings_from_row(row: &sqlx::sqlite::SqliteRow) -> AppResult<AppSettingsDto> {
    Ok(AppSettingsDto {
        onboarding_completed: parse_sqlite_bool(row.try_get("onboarding_completed")?)?,
        active_provider_profile_id: parse_optional_uuid(row, "active_provider_profile_id")?,
        default_learning_profile_id: parse_optional_uuid(row, "default_learning_profile_id")?,
        default_vision_profile_id: parse_optional_uuid(row, "default_vision_profile_id")?,
        theme: parse_theme(&row.try_get::<String, _>("theme")?)?,
        context_mode: parse_context_mode(&row.try_get::<String, _>("context_mode")?)?,
        ui_language: parse_ui_language(&row.try_get::<String, _>("ui_language")?)?,
        ui_language_initialized: parse_sqlite_bool(row.try_get("ui_language_initialized")?)?,
        first_reader_hint_completed: parse_sqlite_bool(
            row.try_get("first_reader_hint_completed")?,
        )?,
    })
}

fn parse_optional_uuid(row: &sqlx::sqlite::SqliteRow, column: &str) -> AppResult<Option<Uuid>> {
    row.try_get::<Option<String>, _>(column)?
        .map(|value| {
            Uuid::parse_str(&value).map_err(|_| AppError::new(AppErrorCode::DatabaseError))
        })
        .transpose()
}

fn parse_sqlite_bool(value: i64) -> AppResult<bool> {
    match value {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(AppError::new(AppErrorCode::DatabaseError)),
    }
}

fn parse_context_mode(value: &str) -> AppResult<ContextMode> {
    match value {
        "standard" => Ok(ContextMode::Standard),
        "long" => Ok(ContextMode::Long),
        _ => Err(AppError::new(AppErrorCode::DatabaseError)),
    }
}

fn parse_ui_language(value: &str) -> AppResult<UiLanguage> {
    match value {
        "zh-CN" => Ok(UiLanguage::ZhCn),
        "zh-TW" => Ok(UiLanguage::ZhTw),
        "en" => Ok(UiLanguage::En),
        _ => Err(AppError::new(AppErrorCode::DatabaseError)),
    }
}

fn ui_language_name(value: UiLanguage) -> &'static str {
    match value {
        UiLanguage::ZhCn => "zh-CN",
        UiLanguage::ZhTw => "zh-TW",
        UiLanguage::En => "en",
    }
}

fn parse_theme(value: &str) -> AppResult<Theme> {
    match value {
        "light" => Ok(Theme::Light),
        "dark" => Ok(Theme::Dark),
        "system" => Ok(Theme::System),
        _ => Err(AppError::new(AppErrorCode::DatabaseError)),
    }
}
fn theme_name(value: &Theme) -> &'static str {
    match value {
        Theme::Light => "light",
        Theme::Dark => "dark",
        Theme::System => "system",
    }
}

pub fn recover_interrupted_imports(pool: &SqlitePool, paths: &AppPaths) -> AppResult<()> {
    let rows = tauri::async_runtime::block_on(async {
        sqlx::query("SELECT id, import_status FROM books")
            .fetch_all(pool)
            .await
            .map_err(AppError::from)
    })?;
    let mut owned_book_ids = HashSet::with_capacity(rows.len());
    let mut incomplete_copy_ids = Vec::new();
    for row in rows {
        let id = Uuid::parse_str(&row.try_get::<String, _>("id")?)
            .map_err(|_| AppError::new(AppErrorCode::DatabaseError))?;
        let status: String = row.try_get("import_status")?;
        owned_book_ids.insert(id);
        if status == "queued" || status == "copying" {
            incomplete_copy_ids.push(id);
        }
    }

    recover_import_storage(paths, &owned_book_ids)?;
    for id in incomplete_copy_ids {
        remove_book_directory(paths, id)?;
    }

    tauri::async_runtime::block_on(async {
        sqlx::query(
            "UPDATE books SET import_status = 'failed', import_error_code = ?, import_error_message = ?, import_error_stage = CASE WHEN import_status IN ('queued', 'copying') THEN 'copying' ELSE import_status END, sha256 = CASE WHEN import_status IN ('queued', 'copying') THEN NULL ELSE sha256 END, stored_path = CASE WHEN import_status IN ('queued', 'copying') THEN NULL ELSE stored_path END, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE import_status IN ('queued', 'copying', 'parsing', 'indexing')",
        )
        .bind("IMPORT_CANCELLED")
        .bind("上次导入因应用退出而中断，请重试")
        .execute(pool)
        .await
        .map_err(AppError::from)?;
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use crate::{
        db::Database,
        domain::{ReaderSettingsDto, UiLanguage},
        errors::AppErrorCode,
    };

    use super::{
        complete_first_reader_hint, get_app_settings, get_reader_settings, initialize_ui_language,
        update_reader_settings, update_ui_language,
    };

    #[test]
    fn ui_language_initialization_is_compare_and_set_and_preferences_are_strict() {
        let temporary = TempDir::new().expect("temporary database");
        let database = Database::open(temporary.path().join("library.sqlite3")).expect("database");

        let defaults = tauri::async_runtime::block_on(get_app_settings(database.pool()))
            .expect("default app settings");
        assert_eq!(defaults.ui_language, UiLanguage::ZhCn);
        assert!(defaults.ui_language_initialized);
        assert!(!defaults.first_reader_hint_completed);

        tauri::async_runtime::block_on(async {
            sqlx::query(
                "UPDATE app_settings SET ui_language = 'en', ui_language_initialized = 0 WHERE id = 1",
            )
            .execute(database.pool())
            .await
            .expect("make the first initialization pending");
            let (first, second) = tokio::join!(
                initialize_ui_language(database.pool(), UiLanguage::ZhCn),
                initialize_ui_language(database.pool(), UiLanguage::ZhTw),
            );
            let winner = get_app_settings(database.pool())
                .await
                .expect("read concurrent initialization winner");
            assert!(winner.ui_language_initialized);
            assert!(matches!(
                winner.ui_language,
                UiLanguage::ZhCn | UiLanguage::ZhTw
            ));
            assert_eq!(first.expect("first initializer result"), winner);
            assert_eq!(second.expect("second initializer result"), winner);
        });

        tauri::async_runtime::block_on(update_ui_language(database.pool(), UiLanguage::En))
            .expect("explicit language update");
        let after_initialize = tauri::async_runtime::block_on(initialize_ui_language(
            database.pool(),
            UiLanguage::ZhTw,
        ))
        .expect("initialization must read the explicit choice");
        assert_eq!(after_initialize.ui_language, UiLanguage::En);

        let completed = tauri::async_runtime::block_on(complete_first_reader_hint(database.pool()))
            .expect("complete reader hint");
        assert!(completed.first_reader_hint_completed);
    }

    #[test]
    fn reader_settings_use_database_defaults_and_enforce_ranges() {
        let temporary = TempDir::new().expect("temporary database");
        let database = Database::open(temporary.path().join("library.sqlite3")).expect("database");
        let defaults = tauri::async_runtime::block_on(get_reader_settings(database.pool()))
            .expect("reader defaults");
        assert_eq!(
            defaults,
            ReaderSettingsDto {
                font_scale: 1.0,
                line_height: 1.6,
                reader_width: 72.0,
                pdf_zoom: 1.0,
                theme: crate::domain::Theme::System,
            }
        );

        let saved = tauri::async_runtime::block_on(update_reader_settings(
            database.pool(),
            ReaderSettingsDto {
                font_scale: 1.25,
                line_height: 1.8,
                reader_width: 80.0,
                pdf_zoom: 1.5,
                theme: crate::domain::Theme::Dark,
            },
        ))
        .expect("valid reader settings");
        assert_eq!(saved.reader_width, 80.0);

        let error = tauri::async_runtime::block_on(update_reader_settings(
            database.pool(),
            ReaderSettingsDto {
                font_scale: 3.0,
                ..saved
            },
        ))
        .expect_err("out-of-range settings must be rejected");
        assert_eq!(error.code, AppErrorCode::InvalidInput);
    }
}
