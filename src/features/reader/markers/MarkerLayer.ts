import type {
  ContentAnchor,
  DocumentLocator,
} from '../../../lib/generated/document';
import type {
  AnnotationMarker,
  MarkerRelocation,
  ReaderAdapter,
} from '../contracts';

export interface MarkerHistoryLabels {
  locate(sequence: number): string;
  open: string;
  edit: string;
  save: string;
  cancel: string;
  description(sequence: number): string;
  unresolved: string;
  saveFailed: string;
}

const defaultLabels: MarkerHistoryLabels = {
  locate: (sequence) => `Locate question ${sequence} in the textbook`,
  open: 'Open answer',
  edit: 'Edit description',
  save: 'Save',
  cancel: 'Cancel',
  description: (sequence) => `Description for question ${sequence}`,
  unresolved: 'The original location could not be restored precisely.',
  saveFailed: 'The description could not be saved.',
};

export class MarkerLayer {
  #generation = 0;
  #adapter: ReaderAdapter | null = null;
  readonly #adapterQueues = new WeakMap<ReaderAdapter, Promise<void>>();
  #markers: readonly AnnotationMarker[] = [];
  #statuses: readonly MarkerRelocation[] = [];

  constructor(
    private readonly historyRoot: HTMLElement | null = null,
    private readonly onActivate: (
      markers: readonly AnnotationMarker[],
    ) => void = () => {},
    private readonly onUpdateSummary: (
      marker: AnnotationMarker,
      summaryText: string,
    ) => Promise<void> = async () => {},
    private labels: MarkerHistoryLabels = defaultLabels,
  ) {}

  setLabels(labels: MarkerHistoryLabels): void {
    this.labels = labels;
    this.renderHistory(this.#markers, this.#statuses);
  }

  async show(
    adapter: ReaderAdapter,
    markers: readonly AnnotationMarker[],
  ): Promise<MarkerRelocation[]> {
    const generation = ++this.#generation;
    const previous = this.#adapter;
    if (previous && previous !== adapter) void previous.showAnnotations([]);
    this.#adapter = adapter;
    this.renderHistory(markers, []);
    const unresolved = markers
      .filter((item) => item.relocationStatus === 'unresolved')
      .map((item) => ({
        annotationId: item.id,
        relocationStatus: 'unresolved' as const,
      }));
    const renderable = markers.filter(
      (item) => item.relocationStatus !== 'unresolved' && item.anchor,
    );
    const execute = async () => {
      if (generation !== this.#generation || adapter !== this.#adapter)
        return [];
      const relocated = await adapter.showAnnotations(renderable);
      if (generation !== this.#generation || adapter !== this.#adapter) {
        await adapter.showAnnotations([]);
        return [];
      }
      const statuses = [...relocated, ...unresolved];
      this.renderHistory(markers, statuses);
      return statuses;
    };
    const previousRender = this.#adapterQueues.get(adapter);
    const render = previousRender
      ? previousRender.catch(() => {}).then(execute)
      : execute();
    this.#adapterQueues.set(
      adapter,
      render.then(
        () => {},
        () => {},
      ),
    );
    return render;
  }

  dispose(): void {
    this.#generation += 1;
    const adapter = this.#adapter;
    this.#adapter = null;
    this.#markers = [];
    this.#statuses = [];
    if (adapter) void adapter.showAnnotations([]);
    this.historyRoot?.replaceChildren();
  }

  private renderHistory(
    markers: readonly AnnotationMarker[],
    statuses: readonly MarkerRelocation[],
  ): void {
    this.#markers = markers;
    this.#statuses = statuses;
    if (!this.historyRoot) return;
    const byId = new Map(
      statuses.map((item) => [item.annotationId, item.relocationStatus]),
    );
    const list = document.createElement('ol');
    list.className = 'reader-marker-history__list';
    for (const marker of markers) {
      const status = byId.get(marker.id) ?? marker.relocationStatus;
      const item = document.createElement('li');
      item.dataset.annotationId = marker.id;
      item.dataset.relocationStatus = status;
      if (marker.kind === 'ai_conversation') {
        const sequence = marker.sequence ?? 1;
        const locate = document.createElement('button');
        locate.type = 'button';
        locate.className = 'reader-marker-history__locate';
        locate.setAttribute('aria-label', this.labels.locate(sequence));
        locate.dataset.markerShape = 'speech';
        locate.dataset.markerPattern = 'stripes';
        locate.textContent = String(sequence);
        locate.disabled = status === 'unresolved' || !marker.anchor;
        locate.addEventListener('click', () => {
          const locator = marker.anchor
            ? locatorFromAnchor(marker.anchor)
            : null;
          if (locator) void this.#adapter?.navigate(locator);
        });
        item.append(locate, this.createQuestionSummary(marker, sequence));
      } else {
        const note = document.createElement('button');
        note.type = 'button';
        note.setAttribute('aria-label', marker.label);
        note.dataset.markerShape = 'note';
        note.dataset.markerPattern = 'dots';
        note.textContent = 'N';
        note.addEventListener('click', () => this.onActivate([marker]));
        item.append(note);
      }
      if (status === 'unresolved') {
        const warning = document.createElement('span');
        warning.className = 'reader-marker-history__warning';
        warning.textContent = this.labels.unresolved;
        item.append(warning);
      }
      list.append(item);
    }
    this.historyRoot.replaceChildren(list);
  }

  private createQuestionSummary(
    marker: AnnotationMarker,
    sequence: number,
  ): HTMLElement {
    const container = document.createElement('div');
    container.className = 'reader-marker-history__summary';
    const text = document.createElement('span');
    text.textContent = marker.summaryText ?? '';
    container.append(text);

    const open = document.createElement('button');
    open.type = 'button';
    open.textContent = this.labels.open;
    open.addEventListener('click', () => this.onActivate([marker]));
    container.append(open);

    const edit = document.createElement('button');
    edit.type = 'button';
    edit.textContent = this.labels.edit;
    edit.addEventListener('click', () => {
      const editor = document.createElement('form');
      editor.className = 'reader-marker-history__editor';
      const input = document.createElement('input');
      input.value = marker.summaryText ?? '';
      input.maxLength = 512;
      input.setAttribute('aria-label', this.labels.description(sequence));
      const save = document.createElement('button');
      save.type = 'submit';
      save.textContent = this.labels.save;
      const cancel = document.createElement('button');
      cancel.type = 'button';
      cancel.textContent = this.labels.cancel;
      const error = document.createElement('span');
      error.setAttribute('role', 'alert');
      cancel.addEventListener('click', () => editor.replaceWith(container));
      editor.addEventListener('submit', (event) => {
        event.preventDefault();
        const next = input.value.trim();
        if (!next) return;
        input.disabled = true;
        save.disabled = true;
        void this.onUpdateSummary(marker, next)
          .then(() => {
            marker.summaryText = next;
            text.textContent = next;
            editor.replaceWith(container);
          })
          .catch(() => {
            input.disabled = false;
            save.disabled = false;
            error.textContent = this.labels.saveFailed;
          });
      });
      editor.append(input, save, cancel, error);
      container.replaceWith(editor);
      input.focus();
    });
    container.append(edit);
    return container;
  }
}

function locatorFromAnchor(anchor: ContentAnchor): DocumentLocator | null {
  if (anchor.kind === 'text') return anchor.selection.locator;
  const locator = anchor.region.locator;
  if (locator.format === 'pdf') {
    return {
      format: 'pdf',
      startPage: locator.page,
      endPage: locator.page,
      rectsByPage: null,
    };
  }
  if (locator.format === 'epub') return locator;
  return {
    format: 'docx',
    startBlockId: locator.blockId,
    startOffset: 0,
    endBlockId: locator.blockId,
    endOffset: 0,
  };
}

export function groupOverlappingMarkers(
  markers: readonly AnnotationMarker[],
): readonly (readonly AnnotationMarker[])[] {
  const groups = new Map<string, AnnotationMarker[]>();
  for (const marker of markers) {
    const key = marker.anchor
      ? JSON.stringify(marker.anchor)
      : `unresolved:${marker.id}`;
    const group = groups.get(key);
    if (group) group.push(marker);
    else groups.set(key, [marker]);
  }
  return [...groups.values()].map((group) => Object.freeze([...group]));
}
