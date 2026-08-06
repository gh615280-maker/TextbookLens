use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub const MAX_SAFE_ACTIVE_OPERATION_COUNT: u32 = 99;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "maintenance.ts")]
pub enum ActiveOperationKind {
    Import,
    Indexing,
    Learning,
    Storage,
}

impl ActiveOperationKind {
    pub const ALL: [Self; 4] = [Self::Import, Self::Indexing, Self::Learning, Self::Storage];

    pub const fn index(self) -> usize {
        match self {
            Self::Import => 0,
            Self::Indexing => 1,
            Self::Learning => 2,
            Self::Storage => 3,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "maintenance.ts")]
pub struct ActiveOperationSummaryDto {
    pub kind: ActiveOperationKind,
    pub count: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "maintenance.ts")]
pub enum MaintenanceStatusCode {
    #[serde(rename = "MAINTENANCE_AVAILABLE")]
    MaintenanceAvailable,
    #[serde(rename = "NORMAL_OPERATIONS_ACTIVE")]
    NormalOperationsActive,
    #[serde(rename = "MAINTENANCE_WAITING")]
    MaintenanceWaiting,
    #[serde(rename = "MAINTENANCE_EXCLUSIVE")]
    MaintenanceExclusive,
    #[serde(rename = "MAINTENANCE_SHUTTING_DOWN")]
    MaintenanceShuttingDown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "maintenance.ts")]
pub struct MaintenanceStatusDto {
    pub code: MaintenanceStatusCode,
    pub active_operations: Vec<ActiveOperationSummaryDto>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "maintenance.ts")]
pub enum MaintenanceErrorCode {
    #[serde(rename = "MAINTENANCE_BUSY")]
    MaintenanceBusy,
    #[serde(rename = "MAINTENANCE_SHUTTING_DOWN")]
    MaintenanceShuttingDown,
    #[serde(rename = "MAINTENANCE_CAPACITY_EXCEEDED")]
    MaintenanceCapacityExceeded,
    #[serde(rename = "STORAGE_ROOT_INVALID")]
    StorageRootInvalid,
    #[serde(rename = "STORAGE_ENTRY_UNSAFE")]
    StorageEntryUnsafe,
    #[serde(rename = "STORAGE_SCAN_FAILED")]
    StorageScanFailed,
    #[serde(rename = "STORAGE_SIZE_OVERFLOW")]
    StorageSizeOverflow,
    #[serde(rename = "APP_DATA_OPEN_FAILED")]
    AppDataOpenFailed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "maintenance.ts")]
pub struct MaintenanceErrorDto {
    pub code: MaintenanceErrorCode,
    pub active_operations: Vec<ActiveOperationSummaryDto>,
}

impl MaintenanceErrorDto {
    pub fn new(code: MaintenanceErrorCode) -> Self {
        Self {
            code,
            active_operations: Vec::new(),
        }
    }

    pub fn busy(status: MaintenanceStatusDto) -> Self {
        Self {
            code: MaintenanceErrorCode::MaintenanceBusy,
            active_operations: status.active_operations,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "maintenance.ts")]
pub enum StorageCategory {
    Source,
    Derived,
    Index,
    Database,
    Cache,
    Log,
}

impl StorageCategory {
    pub const ALL: [Self; 6] = [
        Self::Source,
        Self::Derived,
        Self::Index,
        Self::Database,
        Self::Cache,
        Self::Log,
    ];

    pub const fn index(self) -> usize {
        match self {
            Self::Source => 0,
            Self::Derived => 1,
            Self::Index => 2,
            Self::Database => 3,
            Self::Cache => 4,
            Self::Log => 5,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "maintenance.ts")]
pub struct StorageCategoryUsageDto {
    pub category: StorageCategory,
    pub bytes: u64,
    pub file_count: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "maintenance.ts")]
pub struct StorageUsageDto {
    pub total_bytes: u64,
    pub total_file_count: u64,
    pub categories: Vec<StorageCategoryUsageDto>,
}
