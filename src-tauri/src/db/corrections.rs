use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::{
    domain::{
        IndexCorrectionConflictState, IndexCorrectionReviewDto, IndexCorrectionValueKind,
        NormalizedRect,
    },
    errors::AppResult,
};

use super::indexing::{database_contract_error, parse_timestamp, parse_uuid, to_u32};

pub async fn list_page_corrections(
    pool: &SqlitePool,
    page_id: Uuid,
) -> AppResult<Vec<IndexCorrectionReviewDto>> {
    let rows = sqlx::query(
        "SELECT id, target_block_id, region_x, region_y, region_width, region_height, value_kind, original_value, corrected_value, conflict_state, revision, updated_at FROM index_corrections WHERE page_id = ? ORDER BY created_at, id",
    )
    .bind(page_id.to_string())
    .fetch_all(pool)
    .await?;

    rows.into_iter()
        .map(|row| {
            let region = match (
                row.try_get::<Option<f64>, _>("region_x")?,
                row.try_get::<Option<f64>, _>("region_y")?,
                row.try_get::<Option<f64>, _>("region_width")?,
                row.try_get::<Option<f64>, _>("region_height")?,
            ) {
                (None, None, None, None) => None,
                (Some(x), Some(y), Some(width), Some(height)) => Some(
                    NormalizedRect::new(x, y, width, height)
                        .map_err(|_| database_contract_error())?,
                ),
                _ => return Err(database_contract_error()),
            };
            let value_kind =
                IndexCorrectionValueKind::from_database(&row.try_get::<String, _>("value_kind")?)
                    .ok_or_else(database_contract_error)?;
            let conflict_state = IndexCorrectionConflictState::from_database(
                &row.try_get::<String, _>("conflict_state")?,
            )
            .ok_or_else(database_contract_error)?;

            Ok(IndexCorrectionReviewDto {
                id: parse_uuid(row.try_get::<String, _>("id")?)?,
                target_block_id: parse_uuid(row.try_get::<String, _>("target_block_id")?)?,
                region,
                value_kind,
                original_value: row.try_get("original_value")?,
                corrected_value: row.try_get("corrected_value")?,
                conflict_state,
                revision: to_u32(row.try_get::<i64, _>("revision")?)?,
                updated_at: parse_timestamp(&row.try_get::<String, _>("updated_at")?)?,
            })
        })
        .collect()
}
