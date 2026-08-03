use chrono::{DateTime, SecondsFormat, Utc};
use sqlx::{Row, SqlitePool, sqlite::SqliteRow};

use crate::{
    domain::{
        TeachingInstructionDto, UpdateTeachingInstruction, normalize_teaching_instruction,
        validate_teaching_instruction,
    },
    errors::{AppError, AppErrorCode, AppResult},
};

pub async fn get_teaching_instruction(pool: &SqlitePool) -> AppResult<TeachingInstructionDto> {
    let row = sqlx::query(
        "SELECT instruction, revision, updated_at FROM teaching_preferences WHERE id = 1",
    )
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::new(AppErrorCode::DatabaseError))?;
    teaching_instruction_from_row(&row)
}

pub async fn update_teaching_instruction(
    pool: &SqlitePool,
    update: UpdateTeachingInstruction,
) -> AppResult<TeachingInstructionDto> {
    let instruction = normalize_teaching_instruction(&update.instruction)?;
    let expected_revision = i64::try_from(update.expected_revision)
        .map_err(|_| AppError::new(AppErrorCode::InvalidInput))?;
    let mut transaction = pool.begin().await?;
    let result = sqlx::query(
        "UPDATE teaching_preferences SET instruction = ?, revision = revision + 1, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = 1 AND revision = ?",
    )
    .bind(instruction)
    .bind(expected_revision)
    .execute(&mut *transaction)
    .await?;

    if result.rows_affected() == 0 {
        let singleton_exists: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM teaching_preferences WHERE id = 1")
                .fetch_one(&mut *transaction)
                .await?;
        transaction.rollback().await?;
        return Err(AppError::new(if singleton_exists == 1 {
            AppErrorCode::RequestConflict
        } else {
            AppErrorCode::DatabaseError
        }));
    }
    if result.rows_affected() != 1 {
        transaction.rollback().await?;
        return Err(AppError::new(AppErrorCode::DatabaseError));
    }

    let row = sqlx::query(
        "SELECT instruction, revision, updated_at FROM teaching_preferences WHERE id = 1",
    )
    .fetch_one(&mut *transaction)
    .await?;
    let saved = teaching_instruction_from_row(&row)?;
    transaction.commit().await?;
    Ok(saved)
}

fn teaching_instruction_from_row(row: &SqliteRow) -> AppResult<TeachingInstructionDto> {
    let instruction: String = row.try_get("instruction")?;
    validate_teaching_instruction(&instruction)
        .map_err(|_| AppError::new(AppErrorCode::DatabaseError))?;
    if instruction.chars().all(char::is_whitespace) && !instruction.is_empty() {
        return Err(AppError::new(AppErrorCode::DatabaseError));
    }

    let revision = u64::try_from(row.try_get::<i64, _>("revision")?)
        .map_err(|_| AppError::new(AppErrorCode::DatabaseError))?;
    let updated_at_text: String = row.try_get("updated_at")?;
    let updated_at = DateTime::parse_from_rfc3339(&updated_at_text)
        .map_err(|_| AppError::new(AppErrorCode::DatabaseError))?
        .with_timezone(&Utc);
    if updated_at.to_rfc3339_opts(SecondsFormat::Millis, true) != updated_at_text {
        return Err(AppError::new(AppErrorCode::DatabaseError));
    }

    Ok(TeachingInstructionDto {
        instruction,
        revision,
        updated_at,
    })
}

#[cfg(test)]
mod tests {
    use crate::{db::Database, domain::UpdateTeachingInstruction, errors::AppErrorCode};
    use tempfile::TempDir;

    use super::{get_teaching_instruction, update_teaching_instruction};

    #[test]
    fn teaching_instruction_normalizes_only_line_endings_and_explicit_empty() {
        let temporary = TempDir::new().expect("temporary database");
        let database = Database::open(temporary.path().join("library.sqlite3")).expect("database");

        let initial = tauri::async_runtime::block_on(get_teaching_instruction(database.pool()))
            .expect("initial teaching instruction");
        assert_eq!(initial.instruction, "");
        assert_eq!(initial.revision, 0);

        let saved = tauri::async_runtime::block_on(update_teaching_instruction(
            database.pool(),
            UpdateTeachingInstruction {
                instruction: "  保留  spacing\r\n🙂\tمرحبا\rnext  ".to_owned(),
                expected_revision: 0,
            },
        ))
        .expect("save teaching instruction");
        assert_eq!(saved.instruction, "  保留  spacing\n🙂\tمرحبا\nnext  ");
        assert_eq!(saved.revision, 1);

        let cleared = tauri::async_runtime::block_on(update_teaching_instruction(
            database.pool(),
            UpdateTeachingInstruction {
                instruction: " \t\r\n\u{2003} ".to_owned(),
                expected_revision: 1,
            },
        ))
        .expect("save explicit empty instruction");
        assert_eq!(cleared.instruction, "");
        assert_eq!(cleared.revision, 2);
    }

    #[test]
    fn teaching_instruction_rejects_oversize_and_control_characters() {
        let temporary = TempDir::new().expect("temporary database");
        let database = Database::open(temporary.path().join("library.sqlite3")).expect("database");

        for instruction in ["🙂".repeat(1_001), "visible\u{0000}hidden".to_owned()] {
            let error = tauri::async_runtime::block_on(update_teaching_instruction(
                database.pool(),
                UpdateTeachingInstruction {
                    instruction,
                    expected_revision: 0,
                },
            ))
            .expect_err("invalid instruction must be rejected");
            assert_eq!(error.code, AppErrorCode::InvalidInput);
        }
        let current = tauri::async_runtime::block_on(get_teaching_instruction(database.pool()))
            .expect("unchanged teaching instruction");
        assert_eq!(current.revision, 0);
    }

    #[test]
    fn teaching_instruction_optimistic_concurrency_has_one_winner() {
        let temporary = TempDir::new().expect("temporary database");
        let database = Database::open(temporary.path().join("library.sqlite3")).expect("database");

        tauri::async_runtime::block_on(async {
            let first = update_teaching_instruction(
                database.pool(),
                UpdateTeachingInstruction {
                    instruction: "first synthetic preference".to_owned(),
                    expected_revision: 0,
                },
            );
            let second = update_teaching_instruction(
                database.pool(),
                UpdateTeachingInstruction {
                    instruction: "second synthetic preference".to_owned(),
                    expected_revision: 0,
                },
            );
            let (first, second) = tokio::join!(first, second);
            let outcomes = [first, second];
            assert_eq!(outcomes.iter().filter(|result| result.is_ok()).count(), 1);
            let conflict = outcomes
                .iter()
                .find_map(|result| result.as_ref().err())
                .expect("one stale update");
            assert_eq!(conflict.code, AppErrorCode::RequestConflict);

            let current = get_teaching_instruction(database.pool())
                .await
                .expect("winning instruction");
            assert_eq!(current.revision, 1);
            assert!(
                current.instruction == "first synthetic preference"
                    || current.instruction == "second synthetic preference"
            );
        });
    }
}
