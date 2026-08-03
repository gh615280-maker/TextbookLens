import {
  useEffect,
  useRef,
  useState,
  type CSSProperties,
  type RefObject,
} from 'react';

import type { UiLanguage } from '../../lib/i18n';
import type { DocumentLocator } from '../../lib/generated/document';
import type { ReaderSearchHit, ReaderSection, ReaderSettings } from './api';
import { ReaderFirstHint } from './ReaderFirstHint';
import { ReaderPanel } from './ReaderPanel';
import { ReaderSearch } from './ReaderSearch';
import { ReaderSidebar } from './ReaderSidebar';
import { ReaderToc } from './ReaderToc';
import { ReaderSettingsControls } from './ReaderSettingsControls';
import { ReaderToolbar, type ReaderToolbarLabels } from './ReaderToolbar';
import {
  isReaderFullscreen,
  toggleReaderFullscreen,
} from './reader-fullscreen';

type TransientLayer = 'toc' | 'search' | 'settings';

interface ReaderCopy extends ReaderToolbarLabels {
  content: string;
  document: string;
  close: string;
  emptyToc: string;
  tocDialog: string;
  searchDialog: string;
  settingsDialog: string;
  currentLocation: string;
  markerHistory: string;
  hintTitle: string;
  hintBody: string;
  hintClose: string;
  expand: string;
  collapse: string;
  searchInput: string;
  searchLoading: string;
  searchFailed: string;
  searchEmpty: string;
  currentSection: string;
}

const copy: Record<UiLanguage, ReaderCopy> = {
  'zh-CN': {
    toolbar: '阅读工具栏',
    back: '书库',
    toc: '目录',
    search: '搜索',
    selection: '框选',
    selectionUnavailable: '框选将在后续阶段提供',
    settings: '阅读设置',
    enterFullscreen: '进入全屏',
    exitFullscreen: '退出全屏',
    content: '阅读内容',
    document: '阅读文档',
    close: '关闭',
    emptyToc: '此教材没有可用目录。',
    tocDialog: '教材目录',
    searchDialog: '书内搜索',
    settingsDialog: '阅读设置',
    currentLocation: '当前位置',
    markerHistory: '标记历史',
    hintTitle: '阅读提示',
    hintBody: '选择教材中的文字即可开始学习；此提示只显示一次。',
    hintClose: '知道了',
    expand: '展开',
    collapse: '折叠',
    searchInput: '搜索书内内容',
    searchLoading: '正在搜索…',
    searchFailed: '搜索失败，请重试。',
    searchEmpty: '没有搜索结果。',
    currentSection: '当前章节',
  },
  'zh-TW': {
    toolbar: '閱讀工具列',
    back: '圖書館',
    toc: '目錄',
    search: '搜尋',
    selection: '框選',
    selectionUnavailable: '框選將在後續階段提供',
    settings: '閱讀設定',
    enterFullscreen: '進入全螢幕',
    exitFullscreen: '退出全螢幕',
    content: '閱讀內容',
    document: '閱讀文件',
    close: '關閉',
    emptyToc: '此教材沒有可用目錄。',
    tocDialog: '教材目錄',
    searchDialog: '書內搜尋',
    settingsDialog: '閱讀設定',
    currentLocation: '目前位置',
    markerHistory: '標記歷史',
    hintTitle: '閱讀提示',
    hintBody: '選取教材中的文字即可開始學習；此提示只顯示一次。',
    hintClose: '知道了',
    expand: '展開',
    collapse: '摺疊',
    searchInput: '搜尋書內內容',
    searchLoading: '正在搜尋…',
    searchFailed: '搜尋失敗，請再試一次。',
    searchEmpty: '沒有搜尋結果。',
    currentSection: '目前章節',
  },
  en: {
    toolbar: 'Reader toolbar',
    back: 'Library',
    toc: 'Contents',
    search: 'Search',
    selection: 'Select area',
    selectionUnavailable: 'Area selection will be available in a later phase',
    settings: 'Reading settings',
    enterFullscreen: 'Enter fullscreen',
    exitFullscreen: 'Exit fullscreen',
    content: 'Reading content',
    document: 'Reading document',
    close: 'Close',
    emptyToc: 'This textbook has no available contents.',
    tocDialog: 'Textbook contents',
    searchDialog: 'Search this book',
    settingsDialog: 'Reading settings',
    currentLocation: 'Current location',
    markerHistory: 'Marker history',
    hintTitle: 'Reading tip',
    hintBody:
      'Select text in the book to start learning. This tip appears once.',
    hintClose: 'Got it',
    expand: 'Expand',
    collapse: 'Collapse',
    searchInput: 'Search book content',
    searchLoading: 'Searching…',
    searchFailed: 'Search failed. Try again.',
    searchEmpty: 'No search results.',
    currentSection: 'Current section',
  },
};

interface ReaderLayoutProps {
  title: string;
  language?: UiLanguage;
  location?: string | null;
  firstHintVisible?: boolean;
  onCompleteFirstHint?(): void;
  panelContent?: string;
  settings?: ReaderSettings;
  onSettingsChange?(settings: ReaderSettings): void;
  bookId?: string | null;
  format?: 'pdf' | 'epub' | 'docx';
  sections?: readonly ReaderSection[];
  currentSectionId?: string | null;
  search?(
    bookId: string,
    query: string,
    limit: number,
  ): Promise<ReaderSearchHit[]>;
  onNavigate?(locator: DocumentLocator): Promise<boolean>;
  readerContainerRef?: RefObject<HTMLDivElement | null>;
  markerHistoryRef?: RefObject<HTMLDivElement | null>;
}

export function ReaderLayout({
  title,
  language = 'zh-CN',
  location,
  firstHintVisible = false,
  onCompleteFirstHint,
  panelContent,
  settings,
  onSettingsChange,
  bookId = null,
  format = 'pdf',
  sections = [],
  currentSectionId,
  search,
  onNavigate,
  readerContainerRef,
  markerHistoryRef,
}: ReaderLayoutProps) {
  const labels = copy[language];
  const rootRef = useRef<HTMLDivElement>(null);
  const tocButtonRef = useRef<HTMLButtonElement>(null);
  const searchButtonRef = useRef<HTMLButtonElement>(null);
  const settingsButtonRef = useRef<HTMLButtonElement>(null);
  const [activeLayer, setActiveLayer] = useState<TransientLayer | null>(null);
  const [fullscreen, setFullscreen] = useState(false);

  const triggerFor = (layer: TransientLayer) =>
    layer === 'toc'
      ? tocButtonRef.current
      : layer === 'search'
        ? searchButtonRef.current
        : settingsButtonRef.current;

  const closeLayer = (restoreFocus = true) => {
    if (!activeLayer) return;
    const trigger = triggerFor(activeLayer);
    setActiveLayer(null);
    if (restoreFocus) trigger?.focus();
  };

  const toggleLayer = (layer: TransientLayer) => {
    if (activeLayer === layer) {
      closeLayer();
      return;
    }
    setActiveLayer(layer);
  };

  useEffect(() => {
    if (!activeLayer) return;
    const selector =
      activeLayer === 'search'
        ? '#reader-search-drawer input'
        : `#reader-${activeLayer}-drawer [data-reader-autofocus]`;
    document.querySelector<HTMLElement>(selector)?.focus();
  }, [activeLayer]);

  useEffect(() => {
    const onFullscreenChange = () =>
      setFullscreen(isReaderFullscreen(rootRef.current));
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'F11') {
        event.preventDefault();
        void toggleReaderFullscreen(rootRef.current);
        return;
      }
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'f') {
        event.preventDefault();
        setActiveLayer('search');
        return;
      }
      if (event.key !== 'Escape') return;
      if (activeLayer) {
        event.preventDefault();
        closeLayer();
      } else if (firstHintVisible) {
        event.preventDefault();
        onCompleteFirstHint?.();
      }
    };
    document.addEventListener('fullscreenchange', onFullscreenChange);
    window.addEventListener('keydown', onKeyDown);
    return () => {
      document.removeEventListener('fullscreenchange', onFullscreenChange);
      window.removeEventListener('keydown', onKeyDown);
    };
  });

  const style = settings
    ? ({
        '--reader-font-scale': settings.fontScale,
        '--reader-line-height': settings.lineHeight,
        '--reader-width': `${settings.readerWidth}ch`,
        '--reader-pdf-zoom': settings.pdfZoom,
      } as CSSProperties)
    : undefined;

  const navigate = async (locator: DocumentLocator) => {
    const found = (await onNavigate?.(locator)) ?? false;
    if (found) {
      setActiveLayer(null);
      document.querySelector<HTMLElement>('.reader-main')?.focus();
    }
    return found;
  };

  return (
    <div
      ref={rootRef}
      className={`reader-layout theme-${settings?.theme ?? 'system'}`}
      style={style}
    >
      <header>
        <ReaderToolbar
          labels={labels}
          title={title}
          location={location ?? labels.currentLocation}
          activeLayer={activeLayer}
          fullscreen={fullscreen}
          tocButtonRef={tocButtonRef}
          searchButtonRef={searchButtonRef}
          settingsButtonRef={settingsButtonRef}
          onToggleLayer={toggleLayer}
          onToggleFullscreen={() => {
            void toggleReaderFullscreen(rootRef.current);
          }}
        />
      </header>

      {activeLayer && (
        <div className="reader-transient-backdrop">
          <aside
            id={`reader-${activeLayer}-drawer`}
            className="reader-drawer"
            role="dialog"
            aria-label={
              activeLayer === 'toc'
                ? labels.tocDialog
                : activeLayer === 'search'
                  ? labels.searchDialog
                  : labels.settingsDialog
            }
          >
            <header>
              <h2>
                {activeLayer === 'toc'
                  ? labels.tocDialog
                  : activeLayer === 'search'
                    ? labels.searchDialog
                    : labels.settingsDialog}
              </h2>
              <button
                type="button"
                data-reader-autofocus={
                  activeLayer !== 'search' ? '' : undefined
                }
                onClick={() => closeLayer()}
              >
                {labels.close}
              </button>
            </header>
            {activeLayer === 'toc' &&
              (sections.length > 0 ? (
                <ReaderToc
                  sections={sections}
                  currentSectionId={currentSectionId}
                  label={labels.tocDialog}
                  expandLabel={labels.expand}
                  collapseLabel={labels.collapse}
                  onNavigate={navigate}
                />
              ) : (
                <ReaderSidebar
                  title={labels.tocDialog}
                  emptyMessage={labels.emptyToc}
                />
              ))}
            {activeLayer === 'search' && bookId && search && (
              <ReaderSearch
                bookId={bookId}
                format={format}
                search={search}
                onNavigate={navigate}
                labels={{
                  region: labels.searchDialog,
                  input: labels.searchInput,
                  submit: labels.search,
                  loading: labels.searchLoading,
                  failed: labels.searchFailed,
                  empty: labels.searchEmpty,
                  page: (page) =>
                    language === 'en'
                      ? `Page ${page}`
                      : language === 'zh-TW'
                        ? `第 ${page} 頁`
                        : `第 ${page} 页`,
                  currentSection: labels.currentSection,
                }}
              />
            )}
            {activeLayer === 'settings' && settings && onSettingsChange && (
              <ReaderSettingsControls
                settings={settings}
                format={format}
                language={language}
                onChange={onSettingsChange}
              />
            )}
          </aside>
        </div>
      )}

      {firstHintVisible && onCompleteFirstHint && (
        <ReaderFirstHint
          title={labels.hintTitle}
          body={labels.hintBody}
          closeLabel={labels.hintClose}
          onComplete={onCompleteFirstHint}
        />
      )}

      <main className="reader-main" aria-label={labels.content} tabIndex={-1}>
        <h1>{title}</h1>
        <ReaderPanel content={panelContent} />
        {readerContainerRef ? (
          <div
            ref={readerContainerRef}
            className="reader-document"
            role="region"
            aria-label={labels.document}
          />
        ) : (
          <div
            className="reader-document"
            role="region"
            aria-label={labels.document}
          />
        )}
        {markerHistoryRef && (
          <aside
            className="reader-marker-history"
            aria-label={labels.markerHistory}
          >
            <h2>{labels.markerHistory}</h2>
            <div ref={markerHistoryRef} />
          </aside>
        )}
      </main>
    </div>
  );
}
