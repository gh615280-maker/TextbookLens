import ePub from 'epubjs';
import type { DocumentLocator } from '../../../lib/generated/document';
import type {
  MarkerRelocation,
  NavigationResult,
  ReaderAdapter,
  ReaderAdapterEvents,
  ReaderSource,
  ReadingProgress,
  RegionSelectionOptions,
  RegionSelectionResult,
  SelectionSnapshot,
  AnnotationMarker,
} from '../contracts';
import { groupOverlappingMarkers } from '../markers/MarkerLayer';
import { iframeClientPointToLocal } from '../region-preview-geometry';
import { recoverEpubCfi } from './epub-markers';
import { sanitizeEpubDocument, snapshotEpubRange } from './epub-selection';
import {
  captureEpubRegion,
  hashEpubRegionElement,
  resolveEpubRegionAnchor,
} from './epub-region-capture';
import {
  epubElementRelativeRect,
  epubRegionContainer,
  elementAtEpubPoint,
  EpubRegionSelectionError,
} from './epub-region-selection';
import './epub-reader.css';

type SpineSectionLike = {
  index: number;
  document?: Document;
  load?(loader?: (...args: unknown[]) => Promise<unknown>): Promise<unknown>;
  unload?(): void;
  cfiFromElement?(element: Element): string;
};
type BookLike = {
  open(bytes: ArrayBuffer): Promise<unknown>;
  ready: Promise<unknown>;
  renderTo(element: HTMLElement, options: object): RenditionLike;
  getRange(cfi: string): Promise<Range>;
  load?: (...args: unknown[]) => Promise<unknown>;
  destroy(): void;
  spine: { get(cfi: string): SpineSectionLike | undefined };
  locations?: {
    generate(size: number): Promise<void>;
    percentageFromCfi(cfi: string): number;
  };
};
type RenditionLike = {
  display(target?: string): Promise<unknown>;
  on(name: string, handler: (...args: never[]) => void): void;
  off?(name: string, handler: (...args: never[]) => void): void;
  destroy(): void;
  next?(): Promise<unknown>;
  prev?(): Promise<unknown>;
  annotations: {
    add(
      type: 'highlight',
      cfi: string,
      data?: unknown,
      cb?: () => void,
      className?: string,
    ): void;
    remove?(cfi: string, type: 'highlight'): void;
  };
  hooks: {
    content: {
      register(handler: (contents: EpubContentsLike) => void): void;
    };
  };
};
type EpubContentsLike = {
  document: Document;
  sectionIndex: number;
  cfiFromNode(node: Node): string;
};
type BookFactory = () => BookLike;

export class EpubReaderAdapter implements ReaderAdapter {
  readonly format = 'epub' as const;
  #book: BookLike | null = null;
  #rendition: RenditionLike | null = null;
  #selection: SelectionSnapshot | null = null;
  #cfi = '';
  #generation = 0;
  #locationsReady = false;
  #markerCfis: string[] = [];
  #contents = new Map<Document, EpubContentsLike>();
  #regionAbort: AbortController | null = null;
  #regionReject: ((reason: unknown) => void) | null = null;
  #regionCleanup: (() => void) | null = null;
  #regionAttach: ((contents: EpubContentsLike) => void) | null = null;
  #regionCapture: RegionSelectionResult['capture'] = null;
  #selected = (cfi: string) => {
    void this.captureSelection(cfi);
  };
  #relocated = (location: { start?: { cfi?: string } }) => {
    if (location.start?.cfi) this.#cfi = location.start.cfi;
    this.events.onProgress(this.getProgress());
  };
  #keyDown = (event: KeyboardEvent) => {
    if (event.key === 'ArrowRight') {
      event.preventDefault();
      void this.#rendition?.next?.();
    }
    if (event.key === 'ArrowLeft') {
      event.preventDefault();
      void this.#rendition?.prev?.();
    }
  };
  constructor(
    private readonly container: HTMLElement,
    private readonly events: ReaderAdapterEvents,
    private readonly factory: BookFactory = () =>
      ePub({ replacements: 'none' }) as unknown as BookLike,
  ) {}

  async open(
    source: ReaderSource,
    initial?: DocumentLocator | null,
  ): Promise<void> {
    if (source.kind !== 'document_bytes')
      throw new TypeError('EPUB reader requires in-memory document bytes');
    this.dispose();
    const generation = this.#generation;
    this.container.classList.add('epub-reader');
    const book = this.factory();
    this.#book = book;
    await book.open(source.bytes);
    await book.ready;
    if (generation !== this.#generation || this.#book !== book) {
      book.destroy();
      return;
    }
    const rendition = book.renderTo(this.container, {
      width: '100%',
      height: '100%',
      manager: 'continuous',
      flow: 'scrolled',
    });
    this.#rendition = rendition;
    rendition.hooks.content.register((contents) => {
      if (generation !== this.#generation || rendition !== this.#rendition)
        return;
      sanitizeEpubDocument(contents.document);
      this.#contents.set(contents.document, contents);
      this.#regionAttach?.(contents);
    });
    rendition.on('selected', this.#selected);
    rendition.on('relocated', this.#relocated);
    this.container.addEventListener('keydown', this.#keyDown);
    this.#cfi = initial?.format === 'epub' ? initial.cfi : '';
    await rendition.display(this.#cfi || undefined);
    void book.locations?.generate(1024).then(() => {
      if (generation === this.#generation) this.#locationsReady = true;
    });
  }
  getSelectionSnapshot(): SelectionSnapshot | null {
    return this.#selection;
  }
  async navigate(locator: DocumentLocator): Promise<NavigationResult> {
    if (locator.format !== 'epub' || !this.#rendition) return { found: false };
    await this.#rendition.display(locator.cfi);
    this.#cfi = locator.cfi;
    return { found: true };
  }
  async showAnnotations(
    items: Parameters<ReaderAdapter['showAnnotations']>[0],
  ): Promise<MarkerRelocation[]> {
    for (const cfi of this.#markerCfis) {
      this.#rendition?.annotations.remove?.(cfi, 'highlight');
    }
    this.#markerCfis = [];
    this.container.querySelector('.epub-reader-markers')?.remove();
    const bar = document.createElement('div');
    bar.className = 'epub-reader-markers';
    const statuses: MarkerRelocation[] = [];
    const attached: AnnotationMarker[] = [];
    for (const item of items) {
      const anchor = item.anchor;
      let status: MarkerRelocation['relocationStatus'] = 'unresolved';
      if (
        anchor?.kind === 'text' &&
        anchor.selection.locator.format === 'epub' &&
        this.#rendition &&
        this.#book
      ) {
        const selection = anchor.selection;
        const locator = selection.locator as Extract<
          DocumentLocator,
          { format: 'epub' }
        >;
        try {
          const range = await this.#book.getRange(locator.cfi);
          if (range && rangeMatchesQuote(range, selection.quote.exact)) {
            this.#rendition.annotations.add(
              'highlight',
              locator.cfi,
              { annotationId: item.id },
              undefined,
              'epub-reader-highlight',
            );
            this.#markerCfis.push(locator.cfi);
            status = 'primary';
          }
        } catch {
          /* confined fallback below */
        }
        if (status === 'unresolved') {
          const section = this.#book.spine.get(locator.cfi);
          try {
            await section?.load?.(this.#book.load?.bind(this.#book));
            if (section?.document && section.cfiFromElement) {
              const recovered = recoverEpubCfi(
                {
                  document: section.document,
                  cfiFromElement: section.cfiFromElement.bind(section),
                },
                selection.quote,
              );
              if (recovered) {
                this.#rendition.annotations.add(
                  'highlight',
                  recovered,
                  { annotationId: item.id },
                  undefined,
                  'epub-reader-highlight',
                );
                this.#markerCfis.push(recovered);
                status = 'fallback';
              }
            }
          } finally {
            section?.unload?.();
          }
        }
      } else if (
        anchor?.kind === 'region' &&
        anchor.region.locator.format === 'epub' &&
        this.#rendition &&
        this.#book
      ) {
        const region = anchor.region;
        const locator = region.locator as Extract<
          typeof region.locator,
          { format: 'epub' }
        >;
        const resolver = {
          getRange: (cfi: string) => this.#book!.getRange(cfi),
          sectionIdForCfi: (cfi: string) =>
            exactSectionId(this.#book, cfi) ?? '',
        };
        const primary = await resolveEpubRegionAnchor(resolver, region);
        if (primary) {
          this.#rendition.annotations.add(
            'highlight',
            locator.cfi,
            { annotationId: item.id },
            undefined,
            'epub-reader-highlight',
          );
          this.#markerCfis.push(locator.cfi);
          status = 'primary';
        } else if (region.textFallback) {
          const section = this.#book.spine.get(locator.cfi);
          try {
            await section?.load?.(this.#book.load?.bind(this.#book));
            if (section?.document && section.cfiFromElement) {
              const recovered = recoverEpubCfi(
                {
                  document: section.document,
                  cfiFromElement: section.cfiFromElement.bind(section),
                },
                region.textFallback,
              );
              if (
                recovered &&
                (await resolveEpubRegionAnchor(resolver, {
                  ...region,
                  locator: { ...locator, cfi: recovered },
                }))
              ) {
                this.#rendition.annotations.add(
                  'highlight',
                  recovered,
                  { annotationId: item.id },
                  undefined,
                  'epub-reader-highlight',
                );
                this.#markerCfis.push(recovered);
                status = 'fallback';
              }
            }
          } finally {
            section?.unload?.();
          }
        }
      }
      if (status === 'unresolved') this.events.onFailure(anchorNotFound());
      else attached.push(item);
      statuses.push({ annotationId: item.id, relocationStatus: status });
    }
    for (const group of groupOverlappingMarkers(attached)) {
      bar.append(
        markerButton(group, () => this.events.onMarkerActivate(group)),
      );
    }
    if (bar.childElementCount > 0) this.container.append(bar);
    return statuses;
  }
  async search(): Promise<[]> {
    return [];
  }
  getProgress(): ReadingProgress {
    const fraction =
      this.#cfi && this.#locationsReady
        ? (this.#book?.locations?.percentageFromCfi(this.#cfi) ?? 0)
        : 0;
    return {
      fraction,
      locator: this.#cfi
        ? {
            format: 'epub',
            cfi: this.#cfi,
            sectionId: sectionId(this.#book, this.#cfi),
          }
        : null,
    };
  }

  beginRegionSelection(
    options: RegionSelectionOptions,
  ): Promise<RegionSelectionResult | null> {
    this.cancelRegionSelection();
    if (!this.#rendition || !this.#book)
      return Promise.reject(
        new EpubRegionSelectionError('epub_region_unavailable'),
      );
    const abort = new AbortController();
    this.#regionAbort = abort;
    this.container.classList.add('epub-region-selecting');
    const instruction = Object.assign(document.createElement('div'), {
      className: 'epub-region-instruction',
      textContent:
        'Drag within one EPUB element to select a region. Press Escape to cancel.',
    });
    instruction.setAttribute('role', 'status');
    instruction.setAttribute('aria-live', 'polite');
    this.container.append(instruction);
    return new Promise((resolve, reject) => {
      this.#regionReject = reject;
      const cleanups = new Set<() => void>();
      let finished = false;
      let preview: HTMLElement | null = null;
      let start:
        | {
            contents: EpubContentsLike;
            document: Document;
            element: HTMLElement;
            sectionId: string;
            cfi: string;
            point: { x: number; y: number };
          }
        | undefined;
      const finish = () => {
        if (finished) return;
        finished = true;
        for (const cleanup of cleanups) cleanup();
        cleanups.clear();
        preview?.remove();
        instruction.remove();
        this.container.classList.remove('epub-region-selecting');
        this.#regionAttach = null;
        if (this.#regionAbort === abort) {
          this.#regionAbort = null;
          this.#regionReject = null;
          this.#regionCleanup = null;
        }
      };
      const fail = (
        code: ConstructorParameters<typeof EpubRegionSelectionError>[0],
      ) => {
        if (finished) return;
        abort.abort();
        finish();
        reject(new EpubRegionSelectionError(code));
      };
      const pointTarget = (contents: EpubContentsLike, event: PointerEvent) =>
        epubRegionContainer(event.target, contents.document) ??
        epubRegionContainer(
          elementAtEpubPoint(contents.document, {
            x: event.clientX,
            y: event.clientY,
          }),
          contents.document,
        );
      const previewPoint = (
        contents: EpubContentsLike,
        point: { x: number; y: number },
      ) => {
        const frame = contents.document.defaultView?.frameElement;
        if (!(frame instanceof HTMLElement)) return point;
        return iframeClientPointToLocal(this.container, frame, point);
      };
      const updatePreview = (
        contents: EpubContentsLike,
        point: { x: number; y: number },
      ) => {
        if (!start || !preview) return;
        const first = previewPoint(start.contents, start.point);
        const last = previewPoint(contents, point);
        Object.assign(preview.style, {
          left: `${Math.min(first.x, last.x)}px`,
          top: `${Math.min(first.y, last.y)}px`,
          width: `${Math.abs(last.x - first.x)}px`,
          height: `${Math.abs(last.y - first.y)}px`,
        });
      };
      const onDown = (contents: EpubContentsLike, event: PointerEvent) => {
        if (event.button !== 0) return;
        const element = pointTarget(contents, event);
        if (!element) return;
        const cfi = contents.cfiFromNode(element);
        const currentSection = `spine-${contents.sectionIndex}`;
        if (
          !cfi ||
          exactSectionId(this.#book, cfi) !== currentSection ||
          element.getBoundingClientRect().width <= 0 ||
          element.getBoundingClientRect().height <= 0
        ) {
          fail('epub_region_unstable_container');
          return;
        }
        event.preventDefault();
        start = {
          contents,
          document: contents.document,
          element,
          sectionId: currentSection,
          cfi,
          point: { x: event.clientX, y: event.clientY },
        };
        preview = Object.assign(document.createElement('div'), {
          className: 'epub-region-preview',
        });
        preview.setAttribute('aria-hidden', 'true');
        this.container.append(preview);
        updatePreview(contents, start.point);
      };
      const onMove = (contents: EpubContentsLike, event: PointerEvent) => {
        if (!start) return;
        updatePreview(contents, { x: event.clientX, y: event.clientY });
      };
      const onUp = async (contents: EpubContentsLike, event: PointerEvent) => {
        if (!start) return;
        if (contents !== start.contents) {
          fail('epub_region_cross_section');
          return;
        }
        const endElement = pointTarget(contents, event);
        if (endElement !== start.element) {
          fail('epub_region_unstable_container');
          return;
        }
        let captureToRelease: RegionSelectionResult['capture'] = null;
        try {
          const rect = epubElementRelativeRect(
            start.element.getBoundingClientRect(),
            start.point,
            { x: event.clientX, y: event.clientY },
          );
          const generation = this.#generation;
          const selected = start;
          const result = await captureEpubRegion(
            {
              sectionId: selected.sectionId,
              cfi: selected.cfi,
              element: selected.element,
              rect,
            },
            options.confirmVisualCapture,
            abort.signal,
          );
          captureToRelease = result?.capture ?? null;
          if (generation !== this.#generation || abort.signal.aborted) {
            captureToRelease?.release();
            captureToRelease = null;
            return;
          }
          if (!result) {
            finish();
            resolve(null);
            return;
          }
          const region =
            result.anchor.kind === 'region' && result.anchor.region;
          const currentContents = this.#contents.get(selected.document);
          const currentHash = await hashEpubRegionElement(
            selected.element,
            abort.signal,
          );
          if (
            !region ||
            region.locator.format !== 'epub' ||
            currentContents !== selected.contents ||
            !selected.element.isConnected ||
            exactSectionId(this.#book, selected.cfi) !== selected.sectionId ||
            currentHash !== region.contentSha256
          ) {
            throw new EpubRegionSelectionError('epub_region_content_changed');
          }
          this.#regionCapture?.release();
          this.#regionCapture = result.capture;
          captureToRelease = null;
          finish();
          resolve({
            sectionId: selected.sectionId,
            cfi: selected.cfi,
            rect,
            ...result,
          });
        } catch (error) {
          captureToRelease?.release();
          if (abort.signal.aborted) return;
          finish();
          reject(error);
        }
      };
      const onCancel = () => fail('epub_region_cancelled');
      const onKeyDown = (event: KeyboardEvent) => {
        if (event.key !== 'Escape') return;
        event.preventDefault();
        fail('epub_region_cancelled');
      };
      const attach = (contents: EpubContentsLike) => {
        if (
          [...cleanups].some(
            (cleanup) =>
              (cleanup as { document?: Document }).document ===
              contents.document,
          )
        )
          return;
        const down = (event: PointerEvent) => onDown(contents, event);
        const move = (event: PointerEvent) => onMove(contents, event);
        const up = (event: PointerEvent) => void onUp(contents, event);
        contents.document.addEventListener('pointerdown', down);
        contents.document.addEventListener('pointermove', move);
        contents.document.addEventListener('pointerup', up);
        contents.document.addEventListener('pointercancel', onCancel);
        contents.document.addEventListener('keydown', onKeyDown);
        const cleanup = Object.assign(
          () => {
            contents.document.removeEventListener('pointerdown', down);
            contents.document.removeEventListener('pointermove', move);
            contents.document.removeEventListener('pointerup', up);
            contents.document.removeEventListener('pointercancel', onCancel);
            contents.document.removeEventListener('keydown', onKeyDown);
          },
          { document: contents.document },
        );
        cleanups.add(cleanup);
      };
      this.#regionAttach = attach;
      for (const contents of this.#contents.values()) attach(contents);
      window.addEventListener('pointercancel', onCancel);
      window.addEventListener('keydown', onKeyDown);
      cleanups.add(() => {
        window.removeEventListener('pointercancel', onCancel);
        window.removeEventListener('keydown', onKeyDown);
      });
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
    this.#regionAttach = null;
    cleanup?.();
    this.container.classList.remove('epub-region-selecting');
    reject?.(new EpubRegionSelectionError('epub_region_cancelled'));
  }

  cancel(): void {
    this.cancelRegionSelection();
  }

  dispose(): void {
    this.cancelRegionSelection();
    this.#regionCapture?.release();
    this.#regionCapture = null;
    this.#generation += 1;
    this.container.removeEventListener('keydown', this.#keyDown);
    this.#rendition?.off?.('selected', this.#selected);
    this.#rendition?.off?.('relocated', this.#relocated);
    this.#rendition?.destroy();
    this.#book?.destroy();
    this.#rendition = null;
    this.#book = null;
    this.#selection = null;
    this.#locationsReady = false;
    this.#markerCfis = [];
    this.#contents.clear();
    this.#regionAttach = null;
    this.container.replaceChildren();
    this.container.classList.remove('epub-reader');
  }
  private async captureSelection(cfi: string): Promise<void> {
    const generation = this.#generation;
    const book = this.#book;
    if (!book) return;
    const range = await book.getRange(cfi);
    if (generation !== this.#generation || book !== this.#book) return;
    const snapshot = snapshotEpubRange(range, sectionId(book, cfi), cfi);
    if (!snapshot) return;
    this.#selection = {
      text: snapshot.text,
      anchor: {
        locator: { format: 'epub', cfi, sectionId: snapshot.sectionId },
        quote: snapshot.quote,
        sectionId: snapshot.sectionId,
      },
    };
    this.events.onSelection(this.#selection);
  }
}
function sectionId(book: BookLike | null, cfi: string): string {
  return `spine-${book?.spine.get(cfi)?.index ?? 0}`;
}
function exactSectionId(book: BookLike | null, cfi: string): string | null {
  const section = book?.spine.get(cfi);
  return section ? `spine-${section.index}` : null;
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

function rangeMatchesQuote(range: Range, exact: string): boolean {
  return (
    range.toString().normalize('NFC').replace(/\s+/gu, ' ').trim() ===
    exact.normalize('NFC').replace(/\s+/gu, ' ').trim()
  );
}
