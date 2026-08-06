use std::fmt;

use sqlx::SqlitePool;
use uuid::Uuid;

use crate::{
    db::overview,
    domain::{ActiveOperationKind, LearningOverview, LearningOverviewErrorCode},
    errors::{AppError, AppErrorCode},
    maintenance::gate::MaintenanceGate,
};

#[derive(Clone)]
pub struct LearningOverviewService {
    pool: SqlitePool,
    maintenance_gate: MaintenanceGate,
}

impl LearningOverviewService {
    pub fn new(pool: SqlitePool, maintenance_gate: MaintenanceGate) -> Self {
        Self {
            pool,
            maintenance_gate,
        }
    }

    pub async fn get(
        &self,
        book_id: Uuid,
    ) -> Result<LearningOverview, LearningOverviewServiceError> {
        let _permit = self
            .maintenance_gate
            .try_acquire_normal(ActiveOperationKind::Learning)
            .map_err(|_| LearningOverviewServiceError::Busy)?;
        overview::get_learning_overview(&self.pool, book_id)
            .await
            .map_err(LearningOverviewServiceError::Read)
    }

    #[cfg(test)]
    pub(crate) async fn get_observed<O>(
        &self,
        book_id: Uuid,
        observer: &O,
    ) -> Result<LearningOverview, LearningOverviewServiceError>
    where
        O: overview::OverviewReadObserver + ?Sized,
    {
        let _permit = self
            .maintenance_gate
            .try_acquire_normal(ActiveOperationKind::Learning)
            .map_err(|_| LearningOverviewServiceError::Busy)?;
        overview::get_learning_overview_observed(&self.pool, book_id, observer)
            .await
            .map_err(LearningOverviewServiceError::Read)
    }
}

impl fmt::Debug for LearningOverviewService {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("LearningOverviewService(local-database-only)")
    }
}

pub enum LearningOverviewServiceError {
    Busy,
    Read(AppError),
}

impl LearningOverviewServiceError {
    pub const fn stable_code(&self) -> LearningOverviewErrorCode {
        match self {
            Self::Busy => LearningOverviewErrorCode::Busy,
            Self::Read(error) => match error.code {
                AppErrorCode::InvalidInput => LearningOverviewErrorCode::InvalidInput,
                AppErrorCode::NotFound => LearningOverviewErrorCode::NotFound,
                AppErrorCode::BookNotReady => LearningOverviewErrorCode::BookNotReady,
                _ => LearningOverviewErrorCode::DataInvalid,
            },
        }
    }
}

impl fmt::Debug for LearningOverviewServiceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LearningOverviewServiceError")
            .field("code", &self.stable_code())
            .finish()
    }
}

#[cfg(test)]
#[path = "overview_test.rs"]
mod tests;
