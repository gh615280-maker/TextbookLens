import { useEffect, useMemo, useState } from 'react';
import { useParams } from 'react-router-dom';

import type { ReaderBootstrap, ReaderSection, ReaderSettings } from './api';
import { TauriReaderApi } from './api';
import { ReaderLayout } from './ReaderLayout';

export function ReaderPage() {
  const { bookId } = useParams();
  const api = useMemo(() => new TauriReaderApi(), []);
  const [settings, setSettings] = useState<ReaderSettings | null>(null);
  const [bootstrap, setBootstrap] = useState<ReaderBootstrap | null>(null);
  const [sections, setSections] = useState<ReaderSection[]>([]);
  useEffect(() => {
    void api.getReaderSettings().then(setSettings).catch(() => {});
    if (!bookId) return;
    void api.getReaderBootstrap(bookId).then((value) => { setBootstrap(value); return api.listReaderSections(bookId); }).then(setSections).catch(() => { setBootstrap(null); setSections([]); });
  }, [api, bookId]);
  const updateSettings = (next: ReaderSettings) => { void api.updateReaderSettings(next).then(setSettings).catch(() => {}); };
  return <ReaderLayout title={bootstrap?.book.title ?? (bookId ? '阅读教材' : '阅读器')} bookId={bookId ?? null} format={bootstrap?.book.format ?? 'pdf'} sections={sections} settings={settings ?? undefined} search={api.searchBook.bind(api)} onSettingsChange={updateSettings} />;
}
