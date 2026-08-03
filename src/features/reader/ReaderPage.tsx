import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useParams } from 'react-router-dom';

import { useLanguage } from '../../app/LanguageProvider';
import type { DocumentLocator } from '../../lib/generated/document';
import type { AppSettingsDto } from '../../lib/generated/settings';
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
  const { uiLanguage } = useLanguage();
  const api = useMemo(() => new TauriReaderApi(), []);
  const [settings, setSettings] = useState<ReaderSettings | null>(null);
  const [bootstrap, setBootstrap] = useState<ReaderBootstrap | null>(null);
  const [sections, setSections] = useState<ReaderSection[]>([]);
  const [panelContent, setPanelContent] = useState<string>();
  const [currentLocator, setCurrentLocator] = useState<DocumentLocator | null>(
    null,
  );
  const [firstHintVisible, setFirstHintVisible] = useState(false);
  const readerContainerRef = useRef<HTMLDivElement>(null);
  const markerHistoryRef = useRef<HTMLDivElement>(null);
  const controllerRef = useRef<ReaderController>(null);
  const hintCompletionInFlight = useRef(false);

  const completeFirstHint = useCallback(() => {
    if (hintCompletionInFlight.current) return;
    hintCompletionInFlight.current = true;
    void invoke<AppSettingsDto>('complete_first_reader_hint')
      .then((next) => setFirstHintVisible(!next.firstReaderHintCompleted))
      .catch(() => {})
      .finally(() => {
        hintCompletionInFlight.current = false;
      });
  }, []);

  useEffect(() => {
    void api
      .getReaderSettings()
      .then(setSettings)
      .catch(() => {});
    void invoke<AppSettingsDto>('get_app_settings')
      .then((value) => setFirstHintVisible(!value.firstReaderHintCompleted))
      .catch(() => {});
    if (!bookId) return;
    void api
      .getReaderBootstrap(bookId)
      .then((value) => {
        setBootstrap(value);
        setCurrentLocator(value.lastLocator);
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
        onSelection: (selection) => {
          if (selection) completeFirstHint();
        },
        onProgress: (progress) => setCurrentLocator(progress.locator),
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
  }, [api, bookId, completeFirstHint]);
  const updateSettings = (next: ReaderSettings) => {
    void api
      .updateReaderSettings(next)
      .then(setSettings)
      .catch(() => {});
  };
  return (
    <ReaderLayout
      title={bootstrap?.book.title ?? (bookId ? '阅读教材' : '阅读器')}
      language={uiLanguage}
      location={formatLocation(currentLocator, uiLanguage)}
      firstHintVisible={firstHintVisible}
      onCompleteFirstHint={completeFirstHint}
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

function formatLocation(
  locator: DocumentLocator | null,
  language: 'zh-CN' | 'zh-TW' | 'en',
): string | null {
  if (!locator) return null;
  if (locator.format === 'pdf')
    return language === 'en'
      ? `Page ${locator.startPage}`
      : language === 'zh-TW'
        ? `第 ${locator.startPage} 頁`
        : `第 ${locator.startPage} 页`;
  if (locator.format === 'epub')
    return language === 'en'
      ? 'Current section'
      : language === 'zh-TW'
        ? '目前章節'
        : '当前章节';
  return language === 'en'
    ? 'Current block'
    : language === 'zh-TW'
      ? '目前段落'
      : '当前段落';
}
