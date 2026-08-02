import type {
  AnnotationMarker,
  MarkerRelocation,
  ReaderAdapter,
} from '../contracts';

export class MarkerLayer {
  #generation = 0;
  #adapter: ReaderAdapter | null = null;

  constructor(
    private readonly historyRoot: HTMLElement | null = null,
    private readonly onActivate: (annotationId: string) => void = () => {},
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
    const relocated = await adapter.showAnnotations(renderable);
    if (generation !== this.#generation || adapter !== this.#adapter) {
      await adapter.showAnnotations([]);
      return [];
    }
    const statuses = [...relocated, ...unresolved];
    this.renderHistory(markers, statuses);
    return statuses;
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
    for (const marker of markers) {
      const status = byId.get(marker.id) ?? marker.relocationStatus;
      const item = document.createElement('li');
      item.dataset.annotationId = marker.id;
      item.dataset.relocationStatus = status;
      const button = document.createElement('button');
      button.type = 'button';
      button.setAttribute('aria-label', marker.label);
      button.dataset.markerShape =
        marker.kind === 'ai_conversation' ? 'speech' : 'note';
      button.dataset.markerPattern =
        marker.kind === 'ai_conversation' ? 'stripes' : 'dots';
      button.textContent = marker.kind === 'ai_conversation' ? 'AI' : '◆';
      button.addEventListener('click', () => this.onActivate(marker.id));
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
