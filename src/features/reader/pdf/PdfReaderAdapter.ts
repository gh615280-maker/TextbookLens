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
  RegionSelectionOptions,
  RegionSelectionResult,
  SelectionSnapshot,
  MarkerRelocation,
  AnnotationMarker,
} from '../contracts';
import { groupOverlappingMarkers } from '../markers/MarkerLayer';
import {
  recoverPdfSelection,
  selectionFromRange,
  type PageBounds,
} from './pdf-selection';
import { addPdfRectOverlay } from './pdf-markers';
import {
  capturePdfRegion,
  verifyPdfRegionAnchor,
  type PdfViewportLike,
} from './pdf-region-capture';
import {
  pageAtPoint,
  pageRelativeRect,
  PdfRegionSelectionError,
  normalizeForPdfRotation,
} from './pdf-region-selection';
import './pdf-reader.css';

interface PdfDocumentHandle {
  numPages: number;
  cleanup(): Promise<void> | void;
}

function updateRegionPreview(
  preview: HTMLElement,
  pageBounds: DOMRect,
  start: Readonly<{ x: number; y: number }>,
  end: Readonly<{ x: number; y: number }>,
) {
  const left = Math.max(pageBounds.left, Math.min(start.x, end.x));
  const top = Math.max(pageBounds.top, Math.min(start.y, end.y));
  const right = Math.min(pageBounds.right, Math.max(start.x, end.x));
  const bottom = Math.min(pageBounds.bottom, Math.max(start.y, end.y));
  Object.assign(preview.style, {
    left: `${left - pageBounds.left}px`,
    top: `${top - pageBounds.top}px`,
    width: `${Math.max(0, right - left)}px`,
    height: `${Math.max(0, bottom - top)}px`,
  });
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
  getPageView?(index: number): PdfPageViewLike | undefined;
}
interface PdfPageViewLike {
  pdfPage?: {
    getTextContent(): Promise<{ items: unknown[] }>;
  };
  viewport?: PdfViewportLike & { rotation?: number; scale?: number };
  canvas?: HTMLCanvasElement | null;
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
  #regionAbort: AbortController | null = null;
  #regionReject: ((reason: unknown) => void) | null = null;
  #regionCleanup: (() => void) | null = null;
  #regionCapture: RegionSelectionResult['capture'] = null;
  #annotationItems: AnnotationMarker[] = [];
  #annotationsNeedRefresh = false;
  #annotationRefreshFrame: number | null = null;
  #eventBus: EventBus | null = null;
  #onPageRendered = () => {
    if (this.#annotationItems.length) this.#annotationsNeedRefresh = true;
    if (
      !this.#annotationsNeedRefresh ||
      !this.#annotationItems.length ||
      this.#annotationRefreshFrame !== null
    )
      return;
    this.#annotationRefreshFrame = requestAnimationFrame(() => {
      this.#annotationRefreshFrame = null;
      if (this.#annotationsNeedRefresh && this.#annotationItems.length)
        void this.showAnnotations(this.#annotationItems);
    });
  };

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
    this.#eventBus = eventBus;
    eventBus.on('pagerendered', this.#onPageRendered);
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
    this.#annotationItems = [...items];
    this.container
      .querySelectorAll('.pdf-reader-markers,.pdf-marker-overlay')
      .forEach((node) => node.remove());
    const markers = Object.assign(document.createElement('div'), {
      className: 'pdf-reader-markers',
    });
    const statuses: MarkerRelocation[] = [];
    const attached: AnnotationMarker[] = [];
    let needsRefresh = false;
    for (const item of items) {
      const anchor = item.anchor;
      let status: MarkerRelocation['relocationStatus'] = 'unresolved';
      if (
        anchor?.kind === 'text' &&
        anchor.selection.locator.format === 'pdf'
      ) {
        const selection = anchor.selection;
        const locator = selection.locator as Extract<
          DocumentLocator,
          { format: 'pdf' }
        >;
        const entries = Object.entries(locator.rectsByPage ?? {});
        const boundedPages = [
          ...this.container.querySelectorAll<HTMLElement>('[data-page-number]'),
        ];
        const primaryAvailable =
          entries.length > 0 &&
          entries.every(([page]) =>
            this.container.querySelector(`[data-page-number="${page}"]`),
          ) &&
          recoverPdfSelection(
            boundedPages,
            locator.startPage,
            locator.endPage,
            selection.quote,
          ) !== null;
        if (primaryAvailable) {
          for (const [page, rects] of entries) {
            const element = this.container.querySelector<HTMLElement>(
              `[data-page-number="${page}"]`,
            )!;
            addPdfRectOverlay(
              element,
              pageBounds(Number(page), element),
              rects,
            );
          }
          status = 'primary';
        } else {
          const recovered = recoverPdfSelection(
            boundedPages,
            locator.startPage,
            locator.endPage,
            selection.quote,
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
                pageBounds(Number(page), element),
                rects,
              );
            }
            status = 'fallback';
          }
        }
      } else if (
        anchor?.kind === 'region' &&
        anchor.region.locator.format === 'pdf'
      ) {
        const region = anchor.region;
        const regionLocator = region.locator as Extract<
          typeof region.locator,
          { format: 'pdf' }
        >;
        const page = this.container.querySelector<HTMLElement>(
          `[data-page-number="${regionLocator.page}"]`,
        );
        const verified = page && (await verifyPdfRegionAnchor(page, region));
        const persistedVisualLocation =
          page &&
          item.relocationStatus === 'primary' &&
          region.textFallback === null &&
          validPersistedPdfRegion(page, region);
        if (page && (verified || persistedVisualLocation)) {
          addPdfRectOverlay(page, pageBounds(regionLocator.page, page), [
            region.rect,
          ]);
          status = 'primary';
        } else if (
          page &&
          region.textFallback &&
          /^[0-9a-f]{64}$/u.test(region.contentSha256)
        ) {
          const recovered = recoverPdfSelection(
            [page],
            regionLocator.page,
            regionLocator.page,
            region.textFallback,
          );
          const recoveredLocator = recovered?.locator;
          const rects =
            recoveredLocator?.format === 'pdf'
              ? recoveredLocator.rectsByPage?.[regionLocator.page]
              : undefined;
          if (rects?.length) {
            addPdfRectOverlay(
              page,
              {
                page: regionLocator.page,
                ...page.getBoundingClientRect(),
              },
              rects,
            );
            status = 'fallback';
          }
        }
      }
      if (status === 'unresolved') {
        if (pdfAnnotationRenderReady(this.container, anchor))
          this.events.onFailure(anchorNotFound());
        else needsRefresh = true;
      } else attached.push(item);
      statuses.push({ annotationId: item.id, relocationStatus: status });
    }
    for (const group of groupOverlappingMarkers(attached)) {
      markers.append(
        markerButton(group, () => this.events.onMarkerActivate(group)),
      );
    }
    this.#annotationsNeedRefresh = needsRefresh;
    if (
      items.length > 0 &&
      statuses.every((status) => status.relocationStatus !== 'unresolved')
    )
      this.events.onMarkersResolved?.();
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

  beginRegionSelection(
    options: RegionSelectionOptions,
  ): Promise<RegionSelectionResult | null> {
    this.cancelRegionSelection();
    if (!this.#viewer)
      return Promise.reject(
        new PdfRegionSelectionError('pdf_region_unavailable'),
      );
    const abort = new AbortController();
    this.#regionAbort = abort;
    this.container.classList.add('pdf-region-selecting');
    const instruction = Object.assign(document.createElement('div'), {
      className: 'pdf-region-instruction',
      textContent:
        'Drag within one PDF page to select a region. Press Escape to cancel.',
    });
    instruction.setAttribute('role', 'status');
    this.container.append(instruction);
    return new Promise((resolve, reject) => {
      this.#regionReject = reject;
      let start:
        | { page: HTMLElement; pageNumber: number; x: number; y: number }
        | undefined;
      let preview: HTMLElement | undefined;
      const finish = () => {
        this.container.removeEventListener('pointerdown', onDown);
        window.removeEventListener('pointermove', onMove);
        window.removeEventListener('pointerup', onUp);
        window.removeEventListener('pointercancel', onCancel);
        window.removeEventListener('keydown', onKeyDown);
        preview?.remove();
        instruction.remove();
        this.container.classList.remove('pdf-region-selecting');
        if (this.#regionAbort === abort) {
          this.#regionAbort = null;
          this.#regionReject = null;
          this.#regionCleanup = null;
        }
      };
      const fail = (
        code: ConstructorParameters<typeof PdfRegionSelectionError>[0],
      ) => {
        finish();
        reject(new PdfRegionSelectionError(code));
      };
      const onDown = (event: PointerEvent) => {
        if (event.button !== 0) return;
        const page = pageAtPoint(this.container, event.clientX, event.clientY);
        const pageNumber = Number(page?.dataset.pageNumber);
        if (!page || !Number.isInteger(pageNumber) || pageNumber < 1) return;
        event.preventDefault();
        start = { page, pageNumber, x: event.clientX, y: event.clientY };
        preview = Object.assign(document.createElement('div'), {
          className: 'pdf-region-preview',
        });
        page.append(preview);
        updateRegionPreview(
          preview,
          page.getBoundingClientRect(),
          start,
          start,
        );
      };
      const onMove = (event: PointerEvent) => {
        if (!start || !preview) return;
        updateRegionPreview(
          preview,
          start.page.getBoundingClientRect(),
          start,
          { x: event.clientX, y: event.clientY },
        );
      };
      const onUp = async (event: PointerEvent) => {
        if (!start) return;
        const endPage = pageAtPoint(
          this.container,
          event.clientX,
          event.clientY,
        );
        if (endPage !== start.page) {
          fail('pdf_region_cross_page');
          return;
        }
        try {
          const selected = pageRelativeRect(
            start.pageNumber,
            start.page.getBoundingClientRect(),
            { x: start.x, y: start.y },
            { x: event.clientX, y: event.clientY },
          );
          const generation = this.#generation;
          const pageView = this.#viewer?.getPageView?.(selected.page - 1);
          const viewport = pageView?.viewport;
          const pdfPage = pageView?.pdfPage;
          if (!viewport || !pdfPage)
            throw new PdfRegionSelectionError('pdf_region_unavailable');
          const identity = `${viewport.width}:${viewport.height}:${viewport.rotation ?? 0}:${viewport.scale ?? 0}`;
          const anchorRect = normalizeForPdfRotation(
            selected.rect,
            viewport.rotation ?? 0,
          );
          const textContent = await pdfPage.getTextContent();
          if (generation !== this.#generation || abort.signal.aborted) return;
          if (!start.page.isConnected)
            throw new PdfRegionSelectionError('pdf_region_unavailable');
          const current = this.#viewer?.getPageView?.(
            selected.page - 1,
          )?.viewport;
          if (
            !current ||
            `${current.width}:${current.height}:${current.rotation ?? 0}:${current.scale ?? 0}` !==
              identity
          )
            throw new PdfRegionSelectionError('pdf_region_unavailable');
          const result = await capturePdfRegion(
            {
              page: selected.page,
              rect: selected.rect,
              anchorRect,
              viewport,
              textItems: textContent.items as never[],
              canvas: pageView.canvas ?? start.page.querySelector('canvas'),
            },
            options.confirmVisualCapture,
            abort.signal,
          );
          if (generation !== this.#generation || abort.signal.aborted) {
            result?.capture?.release();
            return;
          }
          const finalViewport = this.#viewer?.getPageView?.(
            selected.page - 1,
          )?.viewport;
          if (
            !start.page.isConnected ||
            !finalViewport ||
            `${finalViewport.width}:${finalViewport.height}:${finalViewport.rotation ?? 0}:${finalViewport.scale ?? 0}` !==
              identity
          ) {
            result?.capture?.release();
            throw new PdfRegionSelectionError('pdf_region_unavailable');
          }
          this.#regionCapture?.release();
          this.#regionCapture = result?.capture ?? null;
          finish();
          resolve(
            result
              ? { page: selected.page, rect: anchorRect, ...result }
              : null,
          );
        } catch (error) {
          if (abort.signal.aborted) return;
          finish();
          reject(error);
        }
      };
      const onCancel = () => fail('pdf_region_cancelled');
      const onKeyDown = (event: KeyboardEvent) => {
        if (event.key === 'Escape') {
          event.preventDefault();
          fail('pdf_region_cancelled');
        }
      };
      this.container.addEventListener('pointerdown', onDown);
      window.addEventListener('pointermove', onMove);
      window.addEventListener('pointerup', onUp);
      window.addEventListener('pointercancel', onCancel);
      window.addEventListener('keydown', onKeyDown);
      this.#regionCleanup = finish;
    });
  }

  cancelRegionSelection(): void {
    this.#regionCapture?.release();
    this.#regionCapture = null;
    if (!this.#regionAbort) return;
    this.#regionAbort.abort();
    const reject = this.#regionReject;
    const cleanup = this.#regionCleanup;
    this.#regionAbort = null;
    this.#regionReject = null;
    this.#regionCleanup = null;
    cleanup?.();
    this.container.classList.remove('pdf-region-selecting');
    reject?.(new PdfRegionSelectionError('pdf_region_cancelled'));
  }

  cancel(): void {
    this.cancelRegionSelection();
  }

  dispose(): void {
    this.cancelRegionSelection();
    this.#regionCapture?.release();
    this.#regionCapture = null;
    this.#generation += 1;
    if (this.#annotationRefreshFrame !== null)
      cancelAnimationFrame(this.#annotationRefreshFrame);
    this.#annotationRefreshFrame = null;
    this.#annotationItems = [];
    this.#annotationsNeedRefresh = false;
    this.#eventBus?.off('pagerendered', this.#onPageRendered);
    this.#eventBus = null;
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

function pageBounds(page: number, element: HTMLElement): PageBounds {
  const bounds = element.getBoundingClientRect();
  return {
    page,
    left: bounds.left,
    top: bounds.top,
    width: bounds.width,
    height: bounds.height,
  };
}

function pdfAnnotationRenderReady(
  container: HTMLElement,
  anchor: AnnotationMarker['anchor'],
): boolean {
  if (!anchor) return true;
  let pages: number[] = [];
  if (anchor.kind === 'text' && anchor.selection.locator.format === 'pdf') {
    const locator = anchor.selection.locator;
    pages = Array.from(
      { length: locator.endPage - locator.startPage + 1 },
      (_, index) => locator.startPage + index,
    );
  } else if (
    anchor.kind === 'region' &&
    anchor.region.locator.format === 'pdf'
  ) {
    pages = [anchor.region.locator.page];
  }
  return (
    pages.length > 0 &&
    pages.every(
      (page) =>
        container.querySelector<HTMLElement>(
          `[data-page-number="${page}"][data-loaded="true"]`,
        ) !== null,
    )
  );
}

function validPersistedPdfRegion(
  page: HTMLElement,
  region: Extract<
    NonNullable<AnnotationMarker['anchor']>,
    { kind: 'region' }
  >['region'],
): boolean {
  const rect = region.rect;
  return (
    region.locator.format === 'pdf' &&
    page.dataset.pageNumber === String(region.locator.page) &&
    [rect.x, rect.y, rect.width, rect.height].every(Number.isFinite) &&
    rect.x >= 0 &&
    rect.y >= 0 &&
    rect.width > 0 &&
    rect.height > 0 &&
    rect.x + rect.width <= 1 &&
    rect.y + rect.height <= 1
  );
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
  group: readonly AnnotationMarker[],
  activate: () => void,
): HTMLButtonElement {
  const first = group[0];
  const overlap = group.length > 1;
  const button = Object.assign(document.createElement('button'), {
    type: 'button',
    textContent: overlap
      ? String(group.length)
      : first.kind === 'ai_conversation'
        ? 'AI'
        : 'N',
    className: 'reader-marker-button',
  });
  button.setAttribute(
    'aria-label',
    overlap ? `Open ${group.length} overlapping markers` : first.label,
  );
  button.dataset.annotationId = first.id;
  button.dataset.markerShape = overlap
    ? 'overlap'
    : first.kind === 'ai_conversation'
      ? 'speech'
      : 'note';
  button.dataset.markerPattern = overlap
    ? 'mixed'
    : first.kind === 'ai_conversation'
      ? 'stripes'
      : 'dots';
  button.addEventListener('click', activate);
  return button;
}
