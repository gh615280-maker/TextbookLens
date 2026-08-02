import { useEffect, useMemo, useState } from 'react';
import { useParams } from 'react-router-dom';

import type { ReaderSettings } from './api';
import { TauriReaderApi } from './api';
import { ReaderLayout } from './ReaderLayout';

export function ReaderPage() {
  const { bookId } = useParams();
  const api = useMemo(() => new TauriReaderApi(), []);
  const [settings, setSettings] = useState<ReaderSettings | null>(null);
  useEffect(() => { void api.getReaderSettings().then(setSettings).catch(() => {}); }, [api]);
  const updateSettings = (next: ReaderSettings) => { void api.updateReaderSettings(next).then(setSettings).catch(() => {}); };
  return <ReaderLayout title={bookId ? '阅读教材' : '阅读器'} settings={settings ?? undefined} onSettingsChange={updateSettings} />;
}
