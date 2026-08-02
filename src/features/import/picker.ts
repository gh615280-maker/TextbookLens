import { open } from '@tauri-apps/plugin-dialog';

export async function pickTextbook(): Promise<string | null> {
  const selected = await open({
    multiple: false,
    directory: false,
    filters: [{ name: '教材', extensions: ['pdf', 'epub', 'docx'] }],
  });
  return typeof selected === 'string' ? selected : null;
}
