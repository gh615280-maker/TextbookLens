import { useEffect, useState, type CSSProperties } from 'react';

import type { ReaderSearchHit, ReaderSection, ReaderSettings } from './api';
import type { DocumentLocator } from '../../lib/generated/document';
import { ReaderPanel } from './ReaderPanel';
import { ReaderSearch } from './ReaderSearch';
import { ReaderToc } from './ReaderToc';
import { ReadingSettings } from './ReadingSettings';
import { ReaderToolbar } from './ReaderToolbar';

interface ReaderLayoutProps {
  title: string; initialSelectionActive?: boolean; panelContent?: string; settings?: ReaderSettings; onSettingsChange?(settings: ReaderSettings): void;
  bookId?: string | null; format?: 'pdf' | 'epub' | 'docx'; sections?: readonly ReaderSection[]; currentSectionId?: string | null;
  search?(bookId: string, query: string, limit: number): Promise<ReaderSearchHit[]>; onNavigate?(locator: DocumentLocator): Promise<boolean>;
}

export function ReaderLayout(props: ReaderLayoutProps) {
  const { title, initialSelectionActive = false, panelContent, settings, onSettingsChange, bookId = null, format = 'pdf', sections = [], currentSectionId, search, onNavigate } = props;
  const [leftOpen, setLeftOpen] = useState(true); const [rightOpen, setRightOpen] = useState(true); const [selectionActive, setSelectionActive] = useState(initialSelectionActive);
  useEffect(() => { const onKeyDown = (event: KeyboardEvent) => { if (event.key === 'Escape') setSelectionActive(false); }; window.addEventListener('keydown', onKeyDown); return () => window.removeEventListener('keydown', onKeyDown); }, []);
  const style = settings ? { '--reader-font-scale': settings.fontScale, '--reader-line-height': settings.lineHeight, '--reader-width': `${settings.readerWidth}ch`, '--reader-pdf-zoom': settings.pdfZoom } as CSSProperties : undefined;
  const navigate = async (locator: DocumentLocator) => { const found = await onNavigate?.(locator) ?? false; if (found) document.querySelector<HTMLElement>('.reader-main')?.focus(); return found; };
  return <section className={`reader-layout theme-${settings?.theme ?? 'system'}`} aria-label={title} style={style}>
    <ReaderToolbar leftOpen={leftOpen} rightOpen={rightOpen} onToggleLeft={() => setLeftOpen((value) => !value)} onToggleRight={() => setRightOpen((value) => !value)} />
    <div className="reader-three-pane">
      {leftOpen && (sections.length > 0 ? <ReaderToc sections={sections} currentSectionId={currentSectionId} onNavigate={navigate} /> : <aside className="reader-sidebar" aria-label="目录"><h2>目录</h2></aside>)}
      <main className="reader-main" aria-label="阅读内容" tabIndex={-1}><h1>{title}</h1>{selectionActive && <output aria-label="已选择文本">已选择文本</output>}<p>阅读器将在打开教材后载入内容。</p></main>
      {rightOpen && <div><ReaderPanel content={panelContent} />{bookId && search && <ReaderSearch bookId={bookId} format={format} search={search} onNavigate={navigate} />}{settings && onSettingsChange && <ReadingSettings settings={settings} format={format} onChange={onSettingsChange} />}</div>}
    </div>
  </section>;
}
