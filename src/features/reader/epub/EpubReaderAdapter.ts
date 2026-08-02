import ePub from 'epubjs';
import type { DocumentLocator } from '../../../lib/generated/document';
import type { NavigationResult, ReaderAdapter, ReaderAdapterEvents, ReaderSource, ReadingProgress, SelectionSnapshot } from '../contracts';
import { sanitizeEpubDocument, snapshotEpubRange } from './epub-selection';
import './epub-reader.css';

type BookLike = { open(bytes: ArrayBuffer): Promise<unknown>; ready: Promise<unknown>; renderTo(element: HTMLElement, options: object): RenditionLike; getRange(cfi: string): Promise<Range>; destroy(): void; spine: { get(cfi: string): { index: number } | undefined }; locations?: { generate(size: number): Promise<void>; percentageFromCfi(cfi: string): number } };
type RenditionLike = { display(target?: string): Promise<unknown>; on(name: string, handler: (...args: never[]) => void): void; off?(name: string, handler: (...args: never[]) => void): void; destroy(): void; next?(): Promise<unknown>; prev?(): Promise<unknown>; annotations: { add(type: 'highlight', cfi: string, data?: unknown, cb?: () => void, className?: string): void }; hooks: { content: { register(handler: (contents: { document: Document }) => void): void } } };
type BookFactory = () => BookLike;

export class EpubReaderAdapter implements ReaderAdapter {
  readonly format = 'epub' as const;
  #book: BookLike | null = null; #rendition: RenditionLike | null = null; #selection: SelectionSnapshot | null = null; #cfi = '';
  #generation = 0; #locationsReady = false;
  #selected = (cfi: string) => { void this.captureSelection(cfi); };
  #relocated = (location: { start?: { cfi?: string } }) => { if (location.start?.cfi) this.#cfi = location.start.cfi; this.events.onProgress(this.getProgress()); };
  #keyDown = (event: KeyboardEvent) => { if (event.key === 'ArrowRight') { event.preventDefault(); void this.#rendition?.next?.(); } if (event.key === 'ArrowLeft') { event.preventDefault(); void this.#rendition?.prev?.(); } };
  constructor(private readonly container: HTMLElement, private readonly events: ReaderAdapterEvents, private readonly factory: BookFactory = () => ePub({ replacements: 'none' }) as unknown as BookLike) {}

  async open(source: ReaderSource, initial?: DocumentLocator | null): Promise<void> {
    if (source.kind !== 'document_bytes') throw new TypeError('EPUB reader requires in-memory document bytes');
    this.dispose(); const generation = this.#generation; this.container.classList.add('epub-reader');
    const book = this.factory(); this.#book = book; await book.open(source.bytes); await book.ready;
    if (generation !== this.#generation || this.#book !== book) { book.destroy(); return; }
    const rendition = book.renderTo(this.container, { width: '100%', height: '100%', flow: 'paginated' }); this.#rendition = rendition;
    rendition.hooks.content.register(({ document }) => sanitizeEpubDocument(document));
    rendition.on('selected', this.#selected); rendition.on('relocated', this.#relocated); this.container.addEventListener('keydown', this.#keyDown); this.#cfi = initial?.format === 'epub' ? initial.cfi : '';
    await rendition.display(this.#cfi || undefined);
    void book.locations?.generate(1024).then(() => { if (generation === this.#generation) this.#locationsReady = true; });
  }
  getSelectionSnapshot(): SelectionSnapshot | null { return this.#selection; }
  async navigate(locator: DocumentLocator): Promise<NavigationResult> { if (locator.format !== 'epub' || !this.#rendition) return { found: false }; await this.#rendition.display(locator.cfi); this.#cfi = locator.cfi; return { found: true }; }
  async showAnnotations(items: Parameters<ReaderAdapter['showAnnotations']>[0]): Promise<void> { const bar = document.createElement('div'); bar.className = 'epub-reader-markers'; for (const item of items) { const button = Object.assign(document.createElement('button'), { type: 'button', textContent: item.label }); button.setAttribute('aria-label', item.label); button.addEventListener('click', () => this.events.onMarkerActivate(item.id)); bar.append(button); const locator = item.anchor?.locator; if (locator?.format === 'epub' && this.#rendition) this.#rendition.annotations.add('highlight', locator.cfi, { annotationId: item.id }, undefined, 'epub-reader-highlight'); } this.container.querySelector('.epub-reader-markers')?.remove(); this.container.append(bar); }
  async search(): Promise<[]> { return []; }
  getProgress(): ReadingProgress { const fraction = this.#cfi && this.#locationsReady ? this.#book?.locations?.percentageFromCfi(this.#cfi) ?? 0 : 0; return { fraction, locator: this.#cfi ? { format: 'epub', cfi: this.#cfi, sectionId: sectionId(this.#book, this.#cfi) } : null }; }
  dispose(): void { this.#generation += 1; this.container.removeEventListener('keydown', this.#keyDown); this.#rendition?.off?.('selected', this.#selected); this.#rendition?.off?.('relocated', this.#relocated); this.#rendition?.destroy(); this.#book?.destroy(); this.#rendition = null; this.#book = null; this.#selection = null; this.#locationsReady = false; this.container.replaceChildren(); this.container.classList.remove('epub-reader'); }
  private async captureSelection(cfi: string): Promise<void> { const generation = this.#generation; const book = this.#book; if (!book) return; const range = await book.getRange(cfi); if (generation !== this.#generation || book !== this.#book) return; const snapshot = snapshotEpubRange(range, sectionId(book, cfi), cfi); if (!snapshot) return; this.#selection = { text: snapshot.text, anchor: { locator: { format: 'epub', cfi, sectionId: snapshot.sectionId }, quote: snapshot.quote, sectionId: snapshot.sectionId } }; this.events.onSelection(this.#selection); }
}
function sectionId(book: BookLike | null, cfi: string): string { return `spine-${book?.spine.get(cfi)?.index ?? 0}`; }
