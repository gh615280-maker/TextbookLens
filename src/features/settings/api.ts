import { invoke } from '@tauri-apps/api/core';

import type {
  BackupSummaryDto,
  ClearAllDataSummaryDto,
  MaintenanceStatusDto,
  RestoreBackupSummaryDto,
  StorageUsageDto,
} from '../../lib/generated/maintenance';
import type { ReaderSettings } from '../reader/api';

/** The settings page deliberately accepts only the path-free maintenance DTOs. */
export interface SettingsApi {
  getMaintenanceStatus(): Promise<MaintenanceStatusDto>;
  getStorageUsage(): Promise<StorageUsageDto>;
  openAppDataDirectory(): Promise<void>;
  createLocalBackup(destination: string): Promise<BackupSummaryDto>;
  restoreLocalBackup(archive: string): Promise<RestoreBackupSummaryDto>;
  clearAllTextbookLensData(
    confirmation: string,
  ): Promise<ClearAllDataSummaryDto>;
  getReaderSettings(): Promise<ReaderSettings>;
  updateReaderSettings(settings: ReaderSettings): Promise<ReaderSettings>;
  restartApplication(): Promise<void>;
}

export const tauriSettingsApi: SettingsApi = {
  getMaintenanceStatus: () => invoke('get_maintenance_status'),
  getStorageUsage: () => invoke('get_storage_usage'),
  openAppDataDirectory: () => invoke('open_app_data_directory'),
  createLocalBackup: (destination) =>
    invoke('create_local_backup', { destination }),
  restoreLocalBackup: (archive) => invoke('restore_local_backup', { archive }),
  clearAllTextbookLensData: (confirmation) =>
    invoke('clear_all_textbooklens_data', { confirmation }),
  getReaderSettings: () => invoke('get_reader_settings'),
  updateReaderSettings: (settings) =>
    invoke('update_reader_settings', { settings }),
  // The command terminates the process after the durable maintenance boundary.
  restartApplication: () => invoke('restart_application'),
};
