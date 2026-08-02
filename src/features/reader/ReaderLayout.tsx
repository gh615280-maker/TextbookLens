import { useEffect, useState, type CSSProperties } from 'react';

import type { ReaderSettings } from './api';
import { ReaderPanel } from './ReaderPanel';
import { ReaderSidebar } from './ReaderSidebar';
import { ReaderSettingsControls } from './ReaderSettingsControls';
import { ReaderToolbar } from './ReaderToolbar';

interface ReaderLayoutProps {
  title: string;
  initialSelectionActive?: boolean;
  panelContent?: string;
  settings?: ReaderSettings;
  onSettingsChange?(settings: ReaderSettings): void;
}

export function ReaderLayout({ title, initialSelectionActive = false, panelContent, settings, onSettingsChange }: ReaderLayoutProps) {
  const [leftOpen, setLeftOpen] = useState(true);
  const [rightOpen, setRightOpen] = useState(true);
  const [selectionActive, setSelectionActive] = useState(initialSelectionActive);
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => { if (event.key === 'Escape') setSelectionActive(false); };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, []);

  const style = settings ? {
    '--reader-font-scale': settings.fontScale,
    '--reader-line-height': settings.lineHeight,
    '--reader-width': `${settings.readerWidth}ch`,
    '--reader-pdf-zoom': settings.pdfZoom,
  } as CSSProperties : undefined;
  return <section className="reader-layout" aria-label={title} style={style}>
    <ReaderToolbar leftOpen={leftOpen} rightOpen={rightOpen} onToggleLeft={() => setLeftOpen((value) => !value)} onToggleRight={() => setRightOpen((value) => !value)} />
    <div className="reader-three-pane">
      {leftOpen && <ReaderSidebar />}
      <main className="reader-main" aria-label="阅读内容" tabIndex={-1}>
        <h1>{title}</h1>
        {selectionActive && <output aria-label="已选择文本">已选择文本</output>}
        <p>阅读器将在打开教材后载入内容。</p>
      </main>
      {rightOpen && (
        <div>
          <ReaderPanel content={panelContent} />
          {settings && onSettingsChange && <ReaderSettingsControls settings={settings} onChange={onSettingsChange} />}
        </div>
      )}
    </div>
  </section>;
}
