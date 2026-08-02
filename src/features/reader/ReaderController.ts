import type { UserFacingError } from '../../lib/errors';
import type { BookFormat } from '../../lib/generated/book';

import type { ReaderApi } from './api';
import type { ReaderAdapter, ReaderAdapterEvents, ReaderAdapterFactory, ReaderSource, ReadingProgress } from './contracts';
import type { DocumentLocator } from '../../lib/generated/document';

const invalidInput = (message: string): UserFacingError => ({
  code: 'INVALID_INPUT', message, nextStep: '请返回书库后重试。', diagnosticId: null,
});

export class ReaderController {
  #adapter: ReaderAdapter | null = null;
  #source: ReaderSource | null = null;
  #bookId: string | null = null;
  #pendingProgress: ReturnType<typeof setTimeout> | null = null;
  #latestProgress: { progress: number; locator: DocumentLocator } | null = null;
  readonly #events: ReaderAdapterEvents;

  constructor(
    private readonly api: ReaderApi,
    private readonly factories: Partial<Record<BookFormat, ReaderAdapterFactory>>,
    events: Partial<ReaderAdapterEvents> = {},
  ) {
    this.#events = {
      onSelection: () => {}, onProgress: () => {}, onMarkerActivate: () => {}, onFailure: () => {}, ...events,
    };
  }

  async open(bookId: string): Promise<void> {
    this.dispose();
    try {
      const bootstrap = await this.api.getReaderBootstrap(bookId);
      const factory = this.factories[bootstrap.book.format];
      if (!factory) {
        this.#events.onFailure(invalidInput('此教材格式暂不支持阅读。'));
        return;
      }
      const adapter = factory({ ...this.#events, onProgress: (progress) => { this.#events.onProgress(progress); this.queueProgress(progress); } });
      if (adapter.format !== bootstrap.book.format) {
        this.#events.onFailure(invalidInput('阅读器格式不匹配。'));
        return;
      }
      this.#adapter = adapter;
      this.#bookId = bookId;
      this.#source = bootstrap.book.format === 'docx'
        ? { kind: 'sanitized_html', html: await this.api.readDerivedText(bookId, 'document.html') }
        : { kind: 'document_bytes', bytes: toArrayBuffer(await this.api.readBookSource(bookId)) };
      await adapter.open(this.#source, bootstrap.lastLocator);
    } catch (error) {
      this.#events.onFailure(isUserFacingError(error) ? error : invalidInput('无法打开教材。'));
    } finally {
      this.#source = null;
    }
  }

  dispose(): void {
    this.flushProgress();
    const adapter = this.#adapter;
    this.#adapter = null;
    this.#source = null;
    this.#bookId = null;
    adapter?.dispose();
  }

  sourceForTesting(): ReaderSource | null {
    return this.#source;
  }

  async navigate(locator: DocumentLocator): Promise<boolean> {
    return (await this.#adapter?.navigate(locator))?.found ?? false;
  }

  private queueProgress(progress: ReadingProgress): void {
    if (!this.#bookId || !progress.locator) return;
    this.#latestProgress = { progress: progress.fraction, locator: progress.locator };
    if (this.#pendingProgress) clearTimeout(this.#pendingProgress);
    this.#pendingProgress = setTimeout(() => this.flushProgress(), 750);
  }

  private flushProgress(): void {
    if (this.#pendingProgress) clearTimeout(this.#pendingProgress);
    this.#pendingProgress = null;
    const pending = this.#latestProgress; const bookId = this.#bookId; this.#latestProgress = null;
    if (pending && bookId) void this.api.saveReadingProgress(bookId, pending.progress, pending.locator).catch((error) => this.#events.onFailure(isUserFacingError(error) ? error : invalidInput('无法保存阅读进度。')));
  }
}

function toArrayBuffer(source: Uint8Array | ArrayBuffer): ArrayBuffer {
  if (source instanceof ArrayBuffer) return source;
  return source.buffer.slice(source.byteOffset, source.byteOffset + source.byteLength) as ArrayBuffer;
}

function isUserFacingError(value: unknown): value is UserFacingError {
  return typeof value === 'object' && value !== null && 'code' in value && 'message' in value && 'nextStep' in value;
}
