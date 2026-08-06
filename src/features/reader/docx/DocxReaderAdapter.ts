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
import { snapshotDocxRange, rangeFromDocxLocator } from './docx-selection';
import {
  addDocxRangeOverlay,
  addDocxRegionOverlay,
  recoverDocxRange,
  recoverDocxRegionRange,
} from './docx-markers';
import {
  captureDocxRegion,
  resolveDocxRegionAnchor,
} from './docx-region-capture';
import {
  docxBlockRelativeRect,
  docxRegionBlock,
  DocxRegionSelectionError,
} from './docx-region-selection';
import './docx-reader.css';

/** Renders only the imported, sanitized derived HTML and never reparses it as executable content. */
export class DocxReaderAdapter implements ReaderAdapter {
  readonly format = 'docx' as const;
  #selection: SelectionSnapshot | null = null;
  #generation = 0;
  #regionAbort: AbortController | null = null;
  #regionReject: ((reason: unknown) => void) | null = null;
  #regionCleanup: (() => void) | null = null;
  #regionCapture: RegionSelectionResult['capture'] = null;
  #onMouseUp = () => this.captureSelection();
  constructor(
    private readonly container: HTMLElement,
    private readonly events: ReaderAdapterEvents,
  ) {}
  async open(
    source: ReaderSource,
    initial?: DocumentLocator | null,
  ): Promise<void> {
    if (source.kind !== 'sanitized_html')
      throw new TypeError('DOCX reader requires derived HTML');
    this.dispose();
    this.container.classList.add('docx-reader');
    renderSafeHtml(this.container, source.html);
    this.container.addEventListener('mouseup', this.#onMouseUp);
    if (initial?.format === 'docx') await this.navigate(initial);
  }
  getSelectionSnapshot(): SelectionSnapshot | null {
    return this.#selection;
  }
  async navigate(locator: DocumentLocator): Promise<NavigationResult> {
    if (locator.format !== 'docx') return { found: false };
    const range = rangeFromDocxLocator(this.container, locator);
    if (!range) return { found: false };
    const target = range.startContainer.parentElement ?? this.container;
    target.scrollIntoView?.({ block: 'center' });
    return { found: true };
  }
  async showAnnotations(
    items: Parameters<ReaderAdapter['showAnnotations']>[0],
  ): Promise<MarkerRelocation[]> {
    this.container
      .querySelectorAll('.docx-marker-overlay,.docx-reader-markers')
      .forEach((node) => node.remove());
    const bar = Object.assign(document.createElement('div'), {
      className: 'docx-reader-markers',
    });
    const statuses: MarkerRelocation[] = [];
    const attached: AnnotationMarker[] = [];
    for (const item of items) {
      const anchor = item.anchor;
      let status: MarkerRelocation['relocationStatus'] = 'unresolved';
      if (
        anchor?.kind === 'text' &&
        anchor.selection.locator.format === 'docx'
      ) {
        const selection = anchor.selection;
        const locator = selection.locator as Extract<
          DocumentLocator,
          { format: 'docx' }
        >;
        const primaryCandidate = rangeFromDocxLocator(this.container, locator);
        const primary =
          primaryCandidate &&
          rangeMatchesQuote(primaryCandidate, selection.quote.exact)
            ? primaryCandidate
            : null;
        const range =
          primary ??
          (selection.sectionId
            ? recoverDocxRange(
                this.container,
                selection.sectionId,
                selection.quote,
                locator,
              )
            : null);
        if (range) {
          addDocxRangeOverlay(this.container, range);
          status = primary ? 'primary' : 'fallback';
        }
      } else if (
        anchor?.kind === 'region' &&
        anchor.region.locator.format === 'docx'
      ) {
        const resolved = await resolveDocxRegionAnchor(
          this.container,
          anchor.region,
        );
        if (resolved) {
          addDocxRegionOverlay(this.container, resolved.block, resolved.rect);
          status = 'primary';
        } else {
          const recovered = recoverDocxRegionRange(
            this.container,
            anchor.region,
          );
          if (recovered) {
            addDocxRangeOverlay(this.container, recovered);
            status = 'fallback';
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
    const first = this.container.querySelector<HTMLElement>('[data-block-id]');
    return {
      fraction: this.container.scrollHeight
        ? this.container.scrollTop /
          Math.max(1, this.container.scrollHeight - this.container.clientHeight)
        : 0,
      locator: first
        ? {
            format: 'docx',
            startBlockId: first.dataset.blockId!,
            startOffset: 0,
            endBlockId: first.dataset.blockId!,
            endOffset: 0,
          }
        : null,
    };
  }
  beginRegionSelection(
    options: RegionSelectionOptions,
  ): Promise<RegionSelectionResult | null> {
    this.cancelRegionSelection();
    if (!this.container.classList.contains('docx-reader'))
      return Promise.reject(
        new DocxRegionSelectionError('docx_region_unavailable'),
      );
    const abort = new AbortController();
    this.#regionAbort = abort;
    this.container.classList.add('docx-region-selecting');
    const instruction = Object.assign(document.createElement('div'), {
      className: 'docx-region-instruction',
      textContent:
        'Drag within one DOCX block to select a region. Press Escape to cancel.',
    });
    instruction.setAttribute('role', 'status');
    instruction.setAttribute('aria-live', 'polite');
    this.container.append(instruction);
    return new Promise((resolve, reject) => {
      this.#regionReject = reject;
      let finished = false;
      let start:
        | {
            block: HTMLElement;
            blockId: string;
            x: number;
            y: number;
          }
        | undefined;
      let preview: HTMLElement | null = null;
      const finish = () => {
        if (finished) return;
        finished = true;
        this.container.removeEventListener('pointerdown', onDown);
        window.removeEventListener('pointermove', onMove);
        window.removeEventListener('pointerup', onUp);
        window.removeEventListener('pointercancel', onCancel);
        window.removeEventListener('keydown', onKeyDown);
        preview?.remove();
        instruction.remove();
        this.container.classList.remove('docx-region-selecting');
        if (this.#regionAbort === abort) {
          this.#regionAbort = null;
          this.#regionReject = null;
          this.#regionCleanup = null;
        }
      };
      const fail = (
        code: ConstructorParameters<typeof DocxRegionSelectionError>[0],
      ) => {
        if (finished) return;
        abort.abort();
        finish();
        reject(new DocxRegionSelectionError(code));
      };
      const targetBlock = (event: PointerEvent) =>
        docxRegionBlock(event.target, this.container) ??
        docxRegionBlock(
          document.elementFromPoint?.(event.clientX, event.clientY),
          this.container,
        );
      const onDown = (event: PointerEvent) => {
        if (event.button !== 0) return;
        const block = targetBlock(event);
        const blockId = block?.dataset.blockId;
        if (!block || !blockId) return;
        const bounds = block.getBoundingClientRect();
        if (bounds.width <= 0 || bounds.height <= 0) {
          fail('docx_region_unavailable');
          return;
        }
        event.preventDefault();
        start = { block, blockId, x: event.clientX, y: event.clientY };
        preview = Object.assign(document.createElement('div'), {
          className: 'docx-region-preview',
        });
        preview.setAttribute('aria-hidden', 'true');
        this.container.append(preview);
        updatePreview(event.clientX, event.clientY);
      };
      const updatePreview = (x: number, y: number) => {
        if (!start || !preview) return;
        const root = this.container.getBoundingClientRect();
        Object.assign(preview.style, {
          left: `${Math.min(start.x, x) - root.left}px`,
          top: `${Math.min(start.y, y) - root.top}px`,
          width: `${Math.abs(x - start.x)}px`,
          height: `${Math.abs(y - start.y)}px`,
        });
      };
      const onMove = (event: PointerEvent) => {
        if (start) updatePreview(event.clientX, event.clientY);
      };
      const onUp = async (event: PointerEvent) => {
        if (!start) return;
        const selected = start;
        const endBlock = targetBlock(event);
        if (endBlock !== selected.block) {
          fail('docx_region_cross_block');
          return;
        }
        let captureToRelease: RegionSelectionResult['capture'] = null;
        try {
          const rect = docxBlockRelativeRect(
            selected.block.getBoundingClientRect(),
            { x: selected.x, y: selected.y },
            { x: event.clientX, y: event.clientY },
          );
          const generation = this.#generation;
          const result = await captureDocxRegion(
            { blockId: selected.blockId, block: selected.block, rect },
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
          const relocated = region
            ? await resolveDocxRegionAnchor(
                this.container,
                region,
                abort.signal,
              )
            : null;
          if (!relocated) {
            throw new DocxRegionSelectionError('docx_region_content_changed');
          }
          this.#regionCapture?.release();
          this.#regionCapture = result.capture;
          captureToRelease = null;
          finish();
          resolve({ blockId: selected.blockId, rect, ...result });
        } catch (error) {
          captureToRelease?.release();
          if (abort.signal.aborted) return;
          finish();
          reject(error);
        }
      };
      const onCancel = () => fail('docx_region_cancelled');
      const onKeyDown = (event: KeyboardEvent) => {
        if (event.key !== 'Escape') return;
        event.preventDefault();
        fail('docx_region_cancelled');
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
    this.container.classList.remove('docx-region-selecting');
    reject?.(new DocxRegionSelectionError('docx_region_cancelled'));
  }

  cancel(): void {
    this.cancelRegionSelection();
  }

  dispose(): void {
    this.cancelRegionSelection();
    this.#regionCapture?.release();
    this.#regionCapture = null;
    this.#generation += 1;
    this.container.removeEventListener('mouseup', this.#onMouseUp);
    this.#selection = null;
    this.container.replaceChildren();
    this.container.classList.remove('docx-reader');
  }
  private captureSelection(): void {
    const selection = window.getSelection();
    if (!selection?.rangeCount) return;
    const snapshot = snapshotDocxRange(selection.getRangeAt(0), this.container);
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
function renderSafeHtml(container: HTMLElement, html: string): void {
  const parsed = new DOMParser().parseFromString(html, 'text/html');
  parsed
    .querySelectorAll('script,iframe,object,embed,form,link,style')
    .forEach((node) => node.remove());
  parsed.querySelectorAll<HTMLElement>('*').forEach((element) => {
    for (const attribute of [...element.attributes])
      if (
        attribute.name.toLowerCase().startsWith('on') ||
        attribute.name.toLowerCase().endsWith('href') ||
        /^(?:srcset|poster|background)$/u.test(attribute.name.toLowerCase()) ||
        (attribute.name === 'style' &&
          /(?:url\s*\(|@import|expression\s*\()/iu.test(attribute.value)) ||
        (attribute.name === 'src' &&
          !/^data:image\/(?:png|jpeg|gif|webp);base64,[a-z0-9+/]+={0,2}$/iu.test(
            attribute.value,
          ))
      )
        element.removeAttribute(attribute.name);
  });
  container.replaceChildren(
    ...[...parsed.body.childNodes].map((node) =>
      document.importNode(node, true),
    ),
  );
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
