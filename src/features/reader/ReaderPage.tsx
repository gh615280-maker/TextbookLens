import { useEffect, useMemo, useRef, useState } from 'react';
import { useParams } from 'react-router-dom';

import type { ReaderBootstrap, ReaderSection, ReaderSettings } from './api';
import { TauriReaderApi } from './api';
import { DocxReaderAdapter } from './docx/DocxReaderAdapter';
import { EpubReaderAdapter } from './epub/EpubReaderAdapter';
import { MarkerLayer } from './markers/MarkerLayer';
import { PdfReaderAdapter } from './pdf/PdfReaderAdapter';
import { ReaderController } from './ReaderController';
import { ReaderLayout } from './ReaderLayout';

export function ReaderPage() {
  const { bookId } = useParams();
  const api = useMemo(() => new TauriReaderApi(), []);
  const [settings, setSettings] = useState<ReaderSettings | null>(null);
  const [bootstrap, setBootstrap] = useState<ReaderBootstrap | null>(null);
  const [sections, setSections] = useState<ReaderSection[]>([]);
  const [panelContent, setPanelContent] = useState<string>();
  const readerContainerRef = useRef<HTMLDivElement>(null);
  const markerHistoryRef = useRef<HTMLDivElement>(null);
  const controllerRef = useRef<ReaderController>(null);
  useEffect(() => {
    void api
      .getReaderSettings()
      .then(setSettings)
      .catch(() => {});
    if (!bookId) return;
    void api
      .getReaderBootstrap(bookId)
      .then((value) => {
        setBootstrap(value);
        return api.listReaderSections(bookId);
      })
      .then(setSections)
      .catch(() => {
        setBootstrap(null);
        setSections([]);
      });
  }, [api, bookId]);
  useEffect(() => {
    const container = readerContainerRef.current;
    if (!bookId || !container) return;
    const markerLayer = new MarkerLayer(markerHistoryRef.current, () =>
      setPanelContent('已选择标记'),
    );
    const controller = new ReaderController(
      api,
      {
        pdf: (events) => new PdfReaderAdapter(container, events),
        epub: (events) => new EpubReaderAdapter(container, events),
        docx: (events) => new DocxReaderAdapter(container, events),
      },
      {
        onFailure: (error) => setPanelContent(error.message),
      },
      markerLayer,
    );
    controllerRef.current = controller;
    void controller.open(bookId);
    return () => {
      controller.dispose();
      if (controllerRef.current === controller) controllerRef.current = null;
    };
  }, [api, bookId]);
  const updateSettings = (next: ReaderSettings) => {
    void api
      .updateReaderSettings(next)
      .then(setSettings)
      .catch(() => {});
  };
  return (
    <ReaderLayout
      title={bootstrap?.book.title ?? (bookId ? '阅读教材' : '阅读器')}
      bookId={bookId ?? null}
      format={bootstrap?.book.format ?? 'pdf'}
      sections={sections}
      settings={settings ?? undefined}
      panelContent={panelContent}
      search={api.searchBook.bind(api)}
      onSettingsChange={updateSettings}
      onNavigate={(locator) =>
        controllerRef.current?.navigate(locator) ?? Promise.resolve(false)
      }
      readerContainerRef={readerContainerRef}
      markerHistoryRef={markerHistoryRef}
    />
  );
}
