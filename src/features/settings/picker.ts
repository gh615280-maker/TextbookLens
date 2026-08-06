import { open, save } from '@tauri-apps/plugin-dialog';

const BACKUP_FILTER = [
  { name: 'TextbookLens backup', extensions: ['tlbackup'] },
];

export function pickBackupDestination(): Promise<string | null> {
  return save({
    filters: BACKUP_FILTER,
    defaultPath: 'textbooklens-backup.tlbackup',
  });
}

export async function pickBackupToRestore(): Promise<string | null> {
  const selected = await open({
    multiple: false,
    directory: false,
    filters: BACKUP_FILTER,
  });
  return typeof selected === 'string' ? selected : null;
}
