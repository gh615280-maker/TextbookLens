import type { RefObject } from 'react';

export interface ReaderToolbarLabels {
  toolbar: string;
  back: string;
  toc: string;
  search: string;
  selection: string;
  selectionUnavailable: string;
  settings: string;
  enterFullscreen: string;
  exitFullscreen: string;
}

interface ReaderToolbarProps {
  labels: ReaderToolbarLabels;
  title: string;
  location: string;
  activeLayer: 'toc' | 'search' | 'settings' | null;
  fullscreen: boolean;
  tocButtonRef: RefObject<HTMLButtonElement | null>;
  searchButtonRef: RefObject<HTMLButtonElement | null>;
  settingsButtonRef: RefObject<HTMLButtonElement | null>;
  onToggleLayer(layer: 'toc' | 'search' | 'settings'): void;
  onToggleFullscreen(): void;
}

export function ReaderToolbar({
  labels,
  title,
  location,
  activeLayer,
  fullscreen,
  tocButtonRef,
  searchButtonRef,
  settingsButtonRef,
  onToggleLayer,
  onToggleFullscreen,
}: ReaderToolbarProps) {
  return (
    <div className="reader-toolbar" role="toolbar" aria-label={labels.toolbar}>
      <a className="reader-toolbar__back" href="/library">
        <span aria-hidden="true">←</span> {labels.back}
      </a>
      <button
        ref={tocButtonRef}
        type="button"
        aria-expanded={activeLayer === 'toc'}
        aria-controls="reader-toc-drawer"
        onClick={() => onToggleLayer('toc')}
      >
        {labels.toc}
      </button>
      <div className="reader-toolbar__location" aria-live="polite">
        <strong>{title}</strong>
        <span aria-hidden="true"> · </span>
        <span>{location}</span>
      </div>
      <button
        ref={searchButtonRef}
        type="button"
        aria-expanded={activeLayer === 'search'}
        aria-controls="reader-search-drawer"
        onClick={() => onToggleLayer('search')}
      >
        {labels.search}
      </button>
      <button
        type="button"
        disabled
        title={labels.selectionUnavailable}
        aria-label={`${labels.selection}：${labels.selectionUnavailable}`}
      >
        {labels.selection}
      </button>
      <button
        ref={settingsButtonRef}
        type="button"
        aria-label={labels.settings}
        aria-expanded={activeLayer === 'settings'}
        aria-controls="reader-settings-drawer"
        onClick={() => onToggleLayer('settings')}
      >
        Aa
      </button>
      <button
        type="button"
        aria-pressed={fullscreen}
        aria-label={fullscreen ? labels.exitFullscreen : labels.enterFullscreen}
        onClick={onToggleFullscreen}
      >
        {fullscreen ? '⤢' : '⛶'}
      </button>
    </div>
  );
}
