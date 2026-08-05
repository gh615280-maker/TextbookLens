import type {
  AnnotationMarker,
  MarkerRelocation,
  ReaderAdapter,
} from '../contracts';

export class MarkerLayer {
  #generation = 0;
  #adapter: ReaderAdapter | null = null;
  readonly #adapterQueues = new WeakMap<ReaderAdapter, Promise<void>>();

  constructor(
    private readonly historyRoot: HTMLElement | null = null,
    private readonly onActivate: (
      markers: readonly AnnotationMarker[],
    ) => void = () => {},
  ) {}

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
    if (adapter) void adapter.showAnnotations([]);
    this.historyRoot?.replaceChildren();
  }

  private renderHistory(
    markers: readonly AnnotationMarker[],
    statuses: readonly MarkerRelocation[],
  ): void {
    if (!this.historyRoot) return;
    const byId = new Map(
      statuses.map((item) => [item.annotationId, item.relocationStatus]),
    );
    const list = document.createElement('ul');
    list.className = 'reader-marker-history';
    for (const group of groupOverlappingMarkers(markers)) {
      const marker = group[0];
      const groupStatuses = group.map(
        (entry) => byId.get(entry.id) ?? entry.relocationStatus,
      );
      const status = groupStatuses.every((value) => value === 'unresolved')
        ? 'unresolved'
        : groupStatuses.some((value) => value === 'fallback')
          ? 'fallback'
          : 'primary';
      const item = document.createElement('li');
      item.dataset.annotationId = marker.id;
      item.dataset.relocationStatus = status;
      item.dataset.overlapCount = String(group.length);
      const button = document.createElement('button');
      button.type = 'button';
      button.setAttribute(
        'aria-label',
        group.length === 1
          ? marker.label
          : `Open ${group.length} overlapping markers`,
      );
      button.dataset.markerShape =
        group.length > 1
          ? 'overlap'
          : marker.kind === 'ai_conversation'
            ? 'speech'
            : 'note';
      button.dataset.markerPattern =
        group.length > 1
          ? 'mixed'
          : marker.kind === 'ai_conversation'
            ? 'stripes'
            : 'dots';
      button.textContent =
        group.length > 1
          ? String(group.length)
          : marker.kind === 'ai_conversation'
            ? 'AI'
            : 'N';
      button.addEventListener('click', () => this.onActivate(group));
      item.append(button);
      if (status === 'unresolved') {
        const warning = document.createElement('span');
        warning.textContent = '原位置无法精确恢复';
        item.append(warning);
      }
      list.append(item);
    }
    this.historyRoot.replaceChildren(list);
  }
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
