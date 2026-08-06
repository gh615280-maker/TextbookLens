use tauri::State;
use uuid::Uuid;

use crate::{
    app_state::AppState,
    domain::{LearningOverview, LearningOverviewErrorDto},
    learning::overview::{LearningOverviewService, LearningOverviewServiceError},
};

#[tauri::command]
pub async fn get_learning_overview(
    state: State<'_, AppState>,
    book_id: String,
) -> Result<LearningOverview, LearningOverviewErrorDto> {
    let book_id = parse_book_id(&book_id)?;
    LearningOverviewService::new(state.db.pool().clone(), state.maintenance_gate.clone())
        .get(book_id)
        .await
        .map_err(stable_error)
}

fn parse_book_id(value: &str) -> Result<Uuid, LearningOverviewErrorDto> {
    Uuid::parse_str(value).map_err(|_| {
        LearningOverviewErrorDto::new(crate::domain::LearningOverviewErrorCode::InvalidInput)
    })
}

fn stable_error(error: LearningOverviewServiceError) -> LearningOverviewErrorDto {
    LearningOverviewErrorDto::new(error.stable_code())
}

#[cfg(test)]
mod tests {
    use crate::{
        domain::{LearningOverviewErrorCode, LearningOverviewErrorDto},
        errors::{AppError, AppErrorCode},
        learning::overview::LearningOverviewServiceError,
    };

    use super::{parse_book_id, stable_error};

    #[test]
    fn command_errors_serialize_as_one_stable_code_only() {
        let cases = [
            (
                LearningOverviewServiceError::Busy,
                LearningOverviewErrorCode::Busy,
            ),
            (
                LearningOverviewServiceError::Read(AppError::new(AppErrorCode::NotFound)),
                LearningOverviewErrorCode::NotFound,
            ),
            (
                LearningOverviewServiceError::Read(AppError::new(AppErrorCode::BookNotReady)),
                LearningOverviewErrorCode::BookNotReady,
            ),
            (
                LearningOverviewServiceError::Read(AppError::database(
                    "PRIVATE_DATABASE_DETAIL_SENTINEL",
                )),
                LearningOverviewErrorCode::DataInvalid,
            ),
        ];
        for (error, expected) in cases {
            let dto = stable_error(error);
            assert_eq!(dto, LearningOverviewErrorDto::new(expected));
            let json = serde_json::to_value(dto).unwrap();
            assert_eq!(json.as_object().unwrap().len(), 1);
            assert!(
                !json
                    .to_string()
                    .contains("PRIVATE_DATABASE_DETAIL_SENTINEL")
            );
        }
        assert_eq!(
            parse_book_id("malformed PRIVATE_BOOK_ID_SENTINEL").unwrap_err(),
            LearningOverviewErrorDto::new(LearningOverviewErrorCode::InvalidInput)
        );
    }
}
