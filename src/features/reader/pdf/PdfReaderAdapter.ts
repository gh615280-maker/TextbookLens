import type { DocumentLocator } from '../../../lib/generated/document';
import { getDocument } from 'pdfjs-dist/legacy/build/pdf.mjs';
import type { ReaderAdapter, ReaderAdapterEvents, ReaderSource, NavigationResult, ReadingProgress, SelectionSnapshot } from '../contracts';
import { selectionFromRange } from './pdf-selection';
import './pdf-reader.css';

interface PdfDocumentHandle { numPages: number; cleanup(): Promise<void> | void; }
interface PdfLoadingTask { promise: Promise<PdfDocumentHandle>; destroy(): Promise<void> | void; }
type PdfLoader = (bytes: ArrayBuffer) => PdfLoadingTask;

/** PDF.js adapter. It accepts only copied bytes, never a URL, and tears down every owned listener/resource. */
export class PdfReaderAdapter implements ReaderAdapter {
  readonly format = 'pdf' as const;
  #document: PdfDocumentHandle | null = null;
  #task: PdfLoadingTask | null = null;
  #selection: SelectionSnapshot | null = null;
  #page = 1;
  #onMouseUp = () => this.captureSelection();

  constructor(private readonly container: HTMLElement, private readonly events: ReaderAdapterEvents, private readonly load: PdfLoader = loadPdf) {}

  async open(source: ReaderSource, initial?: DocumentLocator | null): Promise<void> {
    if (source.kind !== 'document_bytes') throw new TypeError('PDF reader requires in-memory document bytes');
    this.dispose();
    this.container.classList.add('pdf-reader');
    this.container.replaceChildren(Object.assign(document.createElement('div'), { className: 'pdf-viewer', inert: true }));
    this.#task = this.load(source.bytes);
    this.#document = await this.#task.promise;
    this.#page = initial?.format === 'pdf' ? initial.startPage : 1;
    this.container.addEventListener('mouseup', this.#onMouseUp);
  }

  getSelectionSnapshot(): SelectionSnapshot | null { return this.#selection; }

  async navigate(locator: DocumentLocator): Promise<NavigationResult> {
    if (locator.format !== 'pdf' || !this.#document || locator.startPage < 1 || locator.startPage > this.#document.numPages) return { found: false };
    this.#page = locator.startPage;
    this.container.querySelector<HTMLElement>(`[data-page-number="${this.#page}"]`)?.scrollIntoView({ block: 'start' });
    return { found: true };
  }

  async showAnnotations(items: Parameters<ReaderAdapter['showAnnotations']>[0]): Promise<void> {
    this.container.querySelector('.pdf-reader-markers')?.remove();
    const markers = Object.assign(document.createElement('div'), { className: 'pdf-reader-markers' });
    for (const item of items) {
      const button = Object.assign(document.createElement('button'), { type: 'button', textContent: item.label });
      button.setAttribute('aria-label', item.label); button.addEventListener('click', () => this.events.onMarkerActivate(item.id)); markers.append(button);
    }
    this.container.append(markers);
  }

  async search(): Promise<[]> { return []; }
  getProgress(): ReadingProgress { return { fraction: this.#document ? this.#page / this.#document.numPages : 0, locator: this.#document ? { format: 'pdf', startPage: this.#page, endPage: this.#page, rectsByPage: null } : null }; }

  dispose(): void {
    this.container.removeEventListener('mouseup', this.#onMouseUp);
    this.#selection = null;
    const documentHandle = this.#document; const task = this.#task;
    this.#document = null; this.#task = null;
    void documentHandle?.cleanup(); void task?.destroy();
    this.container.replaceChildren(); this.container.classList.remove('pdf-reader');
  }

  private captureSelection(): void {
    const selection = window.getSelection();
    if (!selection || selection.rangeCount === 0) return;
    const pages = [...this.container.querySelectorAll<HTMLElement>('[data-page-number]')];
    const snapshot = selectionFromRange(selection.getRangeAt(0), pages);
    if (!snapshot) return;
    this.#selection = { text: snapshot.text, anchor: { locator: snapshot.locator, quote: snapshot.quote, sectionId: snapshot.sectionId } };
    this.events.onSelection(this.#selection);
  }
}

function loadPdf(bytes: ArrayBuffer): PdfLoadingTask {
  // The parser/viewer is deliberately data-only: PDF.js receives no URL and cannot fetch a remote source.
  return getDocument({ data: new Uint8Array(bytes), isEvalSupported: false } as never) as unknown as PdfLoadingTask;
}
