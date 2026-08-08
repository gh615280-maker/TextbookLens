import type { UserFacingError } from '../../lib/errors';
import type { BookFormat } from '../../lib/generated/book';

import type { ReaderApi } from './api';
import type {
  MarkerRelocation,
  ReaderAdapter,
  ReaderAdapterEvents,
  ReaderAdapterFactory,
  RegionSelectionResult,
  ReaderSource,
  ReadingProgress,
} from './contracts';
import type { DocumentLocator } from '../../lib/generated/document';
import { MarkerLayer } from './markers/MarkerLayer';

const invalidInput = (message: string): UserFacingError => ({
  code: 'INVALID_INPUT',
  message,
  nextStep: '请返回书库后重试。',
  diagnosticId: null,
});

export class ReaderController {
  #adapter: ReaderAdapter | null = null;
  #source: ReaderSource | null = null;
  #bookId: string | null = null;
  #pendingProgress: ReturnType<typeof setTimeout> | null = null;
  #latestProgress: { progress: number; locator: DocumentLocator } | null = null;
  #openGeneration = 0;
  #markerRelocations: MarkerRelocation[] = [];
  readonly #markerLayer: MarkerLayer;
  readonly #events: ReaderAdapterEvents;

  constructor(
    private readonly api: ReaderApi,
    private readonly factories: Partial<
      Record<BookFormat, ReaderAdapterFactory>
    >,
    events: Partial<ReaderAdapterEvents> = {},
    markerLayer?: MarkerLayer,
  ) {
    this.#events = {
      onSelection: () => {},
      onProgress: () => {},
      onMarkerActivate: () => {},
      onFailure: () => {},
      ...events,
    };
    this.#markerLayer =
      markerLayer ??
      new MarkerLayer(null, (markers) =>
        this.#events.onMarkerActivate(markers),
      );
  }

  async open(bookId: string): Promise<void> {
    this.dispose();
    const generation = this.#openGeneration;
    try {
      const bootstrap = await this.api.getReaderBootstrap(bookId);
      if (generation !== this.#openGeneration) return;
      const factory = this.factories[bootstrap.book.format];
      if (!factory) {
        this.#events.onFailure(invalidInput('此教材格式暂不支持阅读。'));
        return;
      }
      const adapter = factory({
        ...this.#events,
        onProgress: (progress) => {
          this.#events.onProgress(progress);
          this.queueProgress(progress);
        },
      });
      if (adapter.format !== bootstrap.book.format) {
        adapter.dispose();
        this.#events.onFailure(invalidInput('阅读器格式不匹配。'));
        return;
      }
      this.#adapter = adapter;
      this.#bookId = bookId;
      this.#source =
        bootstrap.book.format === 'docx'
          ? {
              kind: 'sanitized_html',
              html: await this.api.readDerivedText(bookId, 'document.html'),
            }
          : {
              kind: 'document_bytes',
              bytes: toArrayBuffer(await this.api.readBookSource(bookId)),
            };
      if (generation !== this.#openGeneration || adapter !== this.#adapter)
        return;
      await adapter.open(this.#source, bootstrap.lastLocator);
      if (generation !== this.#openGeneration || adapter !== this.#adapter)
        return;
      if (
        bootstrap.book.format === 'pdf' &&
        this.api.ensurePdfPageSections &&
        adapter.getPageCount
      ) {
        const pageCount = adapter.getPageCount();
        if (pageCount !== null) {
          await this.api.ensurePdfPageSections(bookId, pageCount);
          if (generation !== this.#openGeneration || adapter !== this.#adapter)
            return;
        }
      }
      try {
        const markerDtos = await this.api.listAnnotationMarkers(bookId);
        if (generation !== this.#openGeneration || adapter !== this.#adapter)
          return;
        const markers = markerDtos.map((item) => ({
          id: item.id,
          kind: item.kind,
          conversationId: item.conversationId,
          label: item.accessibilityLabel,
          anchor: item.anchor ?? undefined,
          relocationStatus: item.relocationStatus,
          sequence: item.sequence,
          summaryText: item.summaryText,
          revision: item.revision,
        }));
        const relocations = await this.#markerLayer.show(adapter, markers);
        if (generation === this.#openGeneration && adapter === this.#adapter)
          this.#markerRelocations = relocations;
      } catch (error) {
        if (generation === this.#openGeneration && adapter === this.#adapter)
          this.#events.onFailure(
            isUserFacingError(error)
              ? error
              : invalidInput('无法加载阅读标记。'),
          );
      }
    } catch (error) {
      if (generation === this.#openGeneration)
        this.#events.onFailure(
          isUserFacingError(error) ? error : invalidInput('无法打开教材。'),
        );
    } finally {
      if (generation === this.#openGeneration) this.#source = null;
    }
  }

  dispose(): void {
    this.#openGeneration += 1;
    this.flushProgress();
    const adapter = this.#adapter;
    this.#adapter = null;
    this.#source = null;
    this.#bookId = null;
    this.#markerRelocations = [];
    this.#markerLayer.dispose();
    adapter?.cancelRegionSelection?.();
    adapter?.dispose();
  }

  sourceForTesting(): ReaderSource | null {
    return this.#source;
  }

  async navigate(locator: DocumentLocator): Promise<boolean> {
    return (await this.#adapter?.navigate(locator))?.found ?? false;
  }

  getMarkerRelocations(): readonly MarkerRelocation[] {
    return this.#markerRelocations;
  }

  async refreshAnnotations(): Promise<void> {
    const adapter = this.#adapter;
    const bookId = this.#bookId;
    const generation = this.#openGeneration;
    if (!adapter || !bookId) return;
    try {
      const markerDtos = await this.api.listAnnotationMarkers(bookId);
      if (generation !== this.#openGeneration || adapter !== this.#adapter)
        return;
      const markers = markerDtos.map((item) => ({
        id: item.id,
        kind: item.kind,
        conversationId: item.conversationId,
        label: item.accessibilityLabel,
        anchor: item.anchor ?? undefined,
        relocationStatus: item.relocationStatus,
        sequence: item.sequence,
        summaryText: item.summaryText,
        revision: item.revision,
      }));
      const relocations = await this.#markerLayer.show(adapter, markers);
      if (generation === this.#openGeneration && adapter === this.#adapter)
        this.#markerRelocations = relocations;
    } catch (error) {
      if (generation === this.#openGeneration && adapter === this.#adapter)
        this.#events.onFailure(
          isUserFacingError(error)
            ? error
            : invalidInput(
                '\u65e0\u6cd5\u52a0\u8f7d\u9605\u8bfb\u6807\u8bb0\u3002',
              ),
        );
    }
  }

  getSelectionSnapshot() {
    return this.#adapter?.getSelectionSnapshot() ?? null;
  }

  async beginRegionSelection(): Promise<RegionSelectionResult | null> {
    const adapter = this.#adapter;
    const generation = this.#openGeneration;
    if (!adapter?.beginRegionSelection) {
      this.#events.onFailure(invalidInput('区域选择当前不可用。'));
      return null;
    }
    try {
      // Capture bytes remain local until Task 5 authorizes their staging.
      const result = await adapter.beginRegionSelection({
        confirmVisualCapture: () => true,
      });
      if (generation !== this.#openGeneration || adapter !== this.#adapter) {
        result?.capture?.release();
        return null;
      }
      return result;
    } catch (error) {
      const instructionCode =
        error &&
        typeof error === 'object' &&
        'instructionCode' in error &&
        typeof error.instructionCode === 'string'
          ? error.instructionCode
          : 'region_selection_failed';
      this.#events.onFailure(invalidInput(instructionCode));
      return null;
    }
  }

  cancelRegionSelection(): void {
    if (this.#adapter?.cancelRegionSelection) {
      this.#adapter.cancelRegionSelection();
      return;
    }
    this.#adapter?.cancel?.();
  }

  private queueProgress(progress: ReadingProgress): void {
    if (!this.#bookId || !progress.locator) return;
    this.#latestProgress = {
      progress: progress.fraction,
      locator: progress.locator,
    };
    if (this.#pendingProgress) clearTimeout(this.#pendingProgress);
    this.#pendingProgress = setTimeout(() => this.flushProgress(), 750);
  }

  private flushProgress(): void {
    if (this.#pendingProgress) clearTimeout(this.#pendingProgress);
    this.#pendingProgress = null;
    const pending = this.#latestProgress;
    const bookId = this.#bookId;
    this.#latestProgress = null;
    if (pending && bookId)
      void this.api
        .saveReadingProgress(bookId, pending.progress, pending.locator)
        .catch((error) =>
          this.#events.onFailure(
            isUserFacingError(error)
              ? error
              : invalidInput('无法保存阅读进度。'),
          ),
        );
  }
}

function toArrayBuffer(source: Uint8Array | ArrayBuffer): ArrayBuffer {
  if (source instanceof ArrayBuffer) return source;
  return source.buffer.slice(
    source.byteOffset,
    source.byteOffset + source.byteLength,
  ) as ArrayBuffer;
}

function isUserFacingError(value: unknown): value is UserFacingError {
  return (
    typeof value === 'object' &&
    value !== null &&
    'code' in value &&
    'message' in value &&
    'nextStep' in value
  );
}
