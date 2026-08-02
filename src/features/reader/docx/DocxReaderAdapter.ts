import type { DocumentLocator } from '../../../lib/generated/document';
import type {
  MarkerRelocation,
  NavigationResult,
  ReaderAdapter,
  ReaderAdapterEvents,
  ReaderSource,
  ReadingProgress,
  SelectionSnapshot,
} from '../contracts';
import { snapshotDocxRange, rangeFromDocxLocator } from './docx-selection';
import { addDocxRangeOverlay, recoverDocxRange } from './docx-markers';
import './docx-reader.css';

/** Renders only the imported, sanitized derived HTML and never reparses it as executable content. */
export class DocxReaderAdapter implements ReaderAdapter {
  readonly format = 'docx' as const;
  #selection: SelectionSnapshot | null = null;
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
    for (const item of items) {
      const anchor = item.anchor;
      const locator = anchor?.locator;
      let status: MarkerRelocation['relocationStatus'] = 'unresolved';
      if (locator?.format === 'docx' && anchor) {
        const primary = rangeFromDocxLocator(this.container, locator);
        const range =
          primary ??
          (anchor.sectionId
            ? recoverDocxRange(this.container, anchor.sectionId, anchor.quote)
            : null);
        if (range) {
          addDocxRangeOverlay(this.container, range);
          status = primary ? 'primary' : 'fallback';
        }
      }
      if (status === 'unresolved') this.events.onFailure(anchorNotFound());
      else
        bar.append(
          markerButton(item.label, item.kind, () =>
            this.events.onMarkerActivate(item.id),
          ),
        );
      statuses.push({ annotationId: item.id, relocationStatus: status });
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
  dispose(): void {
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
        attribute.name === 'href' ||
        (attribute.name === 'src' && !/^data:image\//iu.test(attribute.value))
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
