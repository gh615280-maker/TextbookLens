import type { DocumentLocator } from '../../../lib/generated/document';
import { getDocument } from 'pdfjs-dist/legacy/build/pdf.mjs';
import {
  EventBus,
  PDFLinkService,
  PDFViewer,
} from 'pdfjs-dist/web/pdf_viewer.mjs';
import 'pdfjs-dist/web/pdf_viewer.css';
import type {
  ReaderAdapter,
  ReaderAdapterEvents,
  ReaderSource,
  NavigationResult,
  ReadingProgress,
  SelectionSnapshot,
  MarkerRelocation,
} from '../contracts';
import { recoverPdfSelection, selectionFromRange } from './pdf-selection';
import { addPdfRectOverlay } from './pdf-markers';
import './pdf-reader.css';

interface PdfDocumentHandle {
  numPages: number;
  cleanup(): Promise<void> | void;
}
interface PdfLoadingTask {
  promise: Promise<PdfDocumentHandle>;
  destroy(): Promise<void> | void;
}
type PdfLoader = (bytes: ArrayBuffer) => PdfLoadingTask;
interface ViewerLike {
  setDocument(document: PdfDocumentHandle): void;
  cleanup(): void;
  readonly firstPagePromise: Promise<unknown> | null;
  currentPageNumber: number;
  currentScale: number;
  pagesRotation: number;
}
type ViewerFactory = (
  container: HTMLDivElement,
  viewer: HTMLDivElement,
  events: EventBus,
  links: PDFLinkService,
) => ViewerLike;

/** PDF.js adapter. It accepts only copied bytes, never a URL, and tears down every owned listener/resource. */
export class PdfReaderAdapter implements ReaderAdapter {
  readonly format = 'pdf' as const;
  #document: PdfDocumentHandle | null = null;
  #task: PdfLoadingTask | null = null;
  #selection: SelectionSnapshot | null = null;
  #page = 1;
  #viewer: ViewerLike | null = null;
  #generation = 0;
  #onMouseUp = () => this.captureSelection();

  constructor(
    private readonly container: HTMLElement,
    private readonly events: ReaderAdapterEvents,
    private readonly load: PdfLoader = loadPdf,
    private readonly createViewer: ViewerFactory = createPdfViewer,
  ) {}

  async open(
    source: ReaderSource,
    initial?: DocumentLocator | null,
  ): Promise<void> {
    if (source.kind !== 'document_bytes')
      throw new TypeError('PDF reader requires in-memory document bytes');
    this.dispose();
    const generation = this.#generation;
    this.container.classList.add('pdf-reader');
    const viewport = Object.assign(document.createElement('div'), {
      className: 'pdf-viewer-container',
    });
    const pages = Object.assign(document.createElement('div'), {
      className: 'pdfViewer pdf-viewer',
    });
    viewport.append(pages);
    this.container.replaceChildren(viewport);
    const eventBus = new EventBus();
    const links = new PDFLinkService({ eventBus, externalLinkTarget: 0 });
    links.externalLinkEnabled = false;
    const viewer = this.createViewer(viewport, pages, eventBus, links);
    this.#viewer = viewer;
    const task = this.load(source.bytes);
    this.#task = task;
    const documentHandle = await task.promise;
    if (generation !== this.#generation || this.#task !== task) {
      void documentHandle.cleanup();
      void task.destroy();
      return;
    }
    this.#document = documentHandle;
    links.setDocument(documentHandle);
    links.setViewer(viewer);
    viewer.setDocument(documentHandle);
    await viewer.firstPagePromise;
    if (
      generation !== this.#generation ||
      this.#task !== task ||
      this.#viewer !== viewer
    )
      return;
    this.#page = initial?.format === 'pdf' ? initial.startPage : 1;
    viewer.currentPageNumber = this.#page;
    eventBus.on('pagechanging', (event: { pageNumber: number }) => {
      this.#page = event.pageNumber;
      this.events.onProgress(this.getProgress());
    });
    this.container.addEventListener('mouseup', this.#onMouseUp);
  }

  getSelectionSnapshot(): SelectionSnapshot | null {
    return this.#selection;
  }

  async navigate(locator: DocumentLocator): Promise<NavigationResult> {
    if (
      locator.format !== 'pdf' ||
      !this.#document ||
      locator.startPage < 1 ||
      locator.startPage > this.#document.numPages
    )
      return { found: false };
    this.#page = locator.startPage;
    if (this.#viewer) this.#viewer.currentPageNumber = this.#page;
    return { found: true };
  }

  async showAnnotations(
    items: Parameters<ReaderAdapter['showAnnotations']>[0],
  ): Promise<MarkerRelocation[]> {
    this.container
      .querySelectorAll('.pdf-reader-markers,.pdf-marker-overlay')
      .forEach((node) => node.remove());
    const markers = Object.assign(document.createElement('div'), {
      className: 'pdf-reader-markers',
    });
    const statuses: MarkerRelocation[] = [];
    for (const item of items) {
      const anchor = item.anchor;
      const locator = anchor?.locator;
      let status: MarkerRelocation['relocationStatus'] = 'unresolved';
      if (locator?.format === 'pdf' && anchor) {
        const entries = Object.entries(locator.rectsByPage ?? {});
        const primaryAvailable =
          entries.length > 0 &&
          entries.every(([page]) =>
            this.container.querySelector(`[data-page-number="${page}"]`),
          );
        if (primaryAvailable) {
          for (const [page, rects] of entries) {
            const element = this.container.querySelector<HTMLElement>(
              `[data-page-number="${page}"]`,
            )!;
            addPdfRectOverlay(
              element,
              { page: Number(page), ...element.getBoundingClientRect() },
              rects,
            );
          }
          status = 'primary';
        } else {
          const pages = [
            ...this.container.querySelectorAll<HTMLElement>(
              '[data-page-number]',
            ),
          ];
          const recovered = recoverPdfSelection(
            pages,
            locator.startPage,
            locator.endPage,
            anchor.quote,
          );
          const recoveredLocator = recovered?.locator;
          if (
            recoveredLocator?.format === 'pdf' &&
            recoveredLocator.rectsByPage
          ) {
            for (const [page, rects] of Object.entries(
              recoveredLocator.rectsByPage,
            )) {
              const element = this.container.querySelector<HTMLElement>(
                `[data-page-number="${page}"]`,
              )!;
              addPdfRectOverlay(
                element,
                { page: Number(page), ...element.getBoundingClientRect() },
                rects,
              );
            }
            status = 'fallback';
          }
        }
      }
      if (status === 'unresolved') this.events.onFailure(anchorNotFound());
      else
        markers.append(
          markerButton(item.label, item.kind, () =>
            this.events.onMarkerActivate(item.id),
          ),
        );
      statuses.push({ annotationId: item.id, relocationStatus: status });
    }
    if (markers.childElementCount > 0) this.container.append(markers);
    return statuses;
  }

  async search(): Promise<[]> {
    return [];
  }
  getProgress(): ReadingProgress {
    return {
      fraction: this.#document ? this.#page / this.#document.numPages : 0,
      locator: this.#document
        ? {
            format: 'pdf',
            startPage: this.#page,
            endPage: this.#page,
            rectsByPage: null,
          }
        : null,
    };
  }

  dispose(): void {
    this.#generation += 1;
    this.container.removeEventListener('mouseup', this.#onMouseUp);
    this.#selection = null;
    const documentHandle = this.#document;
    const task = this.#task;
    this.#document = null;
    this.#task = null;
    this.#viewer?.cleanup();
    this.#viewer = null;
    void documentHandle?.cleanup();
    void task?.destroy();
    this.container.replaceChildren();
    this.container.classList.remove('pdf-reader');
  }

  private captureSelection(): void {
    const selection = window.getSelection();
    if (!selection || selection.rangeCount === 0) return;
    const pages = [
      ...this.container.querySelectorAll<HTMLElement>('[data-page-number]'),
    ];
    const snapshot = selectionFromRange(selection.getRangeAt(0), pages);
    if (!snapshot) return;
    this.#selection = {
      text: snapshot.text,
      anchor: {
        locator: snapshot.locator,
        quote: snapshot.quote,
        sectionId: snapshot.sectionId,
      },
    };
    this.events.onSelection(this.#selection);
  }
}

function loadPdf(bytes: ArrayBuffer): PdfLoadingTask {
  // The parser/viewer is deliberately data-only: PDF.js receives no URL and cannot fetch a remote source.
  return getDocument({
    data: new Uint8Array(bytes),
    isEvalSupported: false,
  } as never) as unknown as PdfLoadingTask;
}

function createPdfViewer(
  container: HTMLDivElement,
  viewer: HTMLDivElement,
  eventBus: EventBus,
  links: PDFLinkService,
): ViewerLike {
  return new PDFViewer({
    container,
    viewer,
    eventBus,
    linkService: links,
    annotationMode: 0,
    enableAutoLinking: false,
    enableSelectionRendering: true,
  });
}
function anchorNotFound() {
  return {
    code: 'ANCHOR_NOT_FOUND' as const,
    message: 'The saved marker could not be located.',
    nextStep: 'Open the marker from history.',
    diagnosticId: null,
  };
}
function markerButton(
  label: string,
  kind: 'ai_conversation' | 'note',
  activate: () => void,
): HTMLButtonElement {
  const button = Object.assign(document.createElement('button'), {
    type: 'button',
    textContent: kind === 'ai_conversation' ? 'AI' : '◆',
  });
  button.setAttribute('aria-label', label);
  button.dataset.markerShape = kind === 'ai_conversation' ? 'speech' : 'note';
  button.dataset.markerPattern =
    kind === 'ai_conversation' ? 'stripes' : 'dots';
  button.addEventListener('click', activate);
  return button;
}
