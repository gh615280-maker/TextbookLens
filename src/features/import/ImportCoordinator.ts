import type { BookFormat, BookSummary } from '../../lib/generated/book';
import type { NormalizedSectionInput } from '../../lib/generated/document';
import { toUserError, type UserFacingError } from '../../lib/errors';
import type { ImportIpc } from '../../lib/ipc';
import { toOwnedArrayBuffer } from '../../lib/ipc';
import { stableBlockId, stableSectionId } from './id';
import type {
  DocumentParser,
  ImportEvent,
  ImportStage,
  ParsedBookMetadata,
  ParserSink,
} from './parser-contract';
import type { ParserRegistry } from './parser-registry';

const MAX_BATCH_SECTIONS = 25;
const MAX_BATCH_BLOCKS = 500;

interface ActiveImport {
  controller: AbortController;
  parser: DocumentParser | undefined;
  source: ArrayBuffer | undefined;
}

export class ImportCoordinator {
  readonly #activeImports = new Map<string, ActiveImport>();

  constructor(
    private readonly ipc: ImportIpc,
    private readonly parsers: ParserRegistry,
  ) {}

  get activeJobCount(): number {
    return this.#activeImports.size;
  }

  async importDocument(
    sourcePath: string,
    onProgress: (event: ImportEvent) => void = () => {},
  ): Promise<BookSummary> {
    let outcome;
    try {
      outcome = await this.ipc.beginImport(sourcePath, onProgress);
    } catch (error) {
      throw toUserError(error);
    }
    if (outcome.outcome === 'duplicate') return outcome.book;

    const { book } = outcome;
    if (this.#activeImports.has(book.id)) {
      throw contractError('REQUEST_CONFLICT');
    }

    const active: ActiveImport = {
      controller: new AbortController(),
      parser: undefined,
      source: undefined,
    };
    this.#activeImports.set(book.id, active);
    let binary: Uint8Array | ArrayBuffer | undefined;
    let source: ArrayBuffer | undefined;
    let selectedParser: DocumentParser | undefined;
    let stage: ImportStage = 'parsing';

    try {
      assertNotAborted(active.controller.signal);
      binary = await this.ipc.readBookSource(book.id);
      assertNotAborted(active.controller.signal);
      source = toOwnedArrayBuffer(binary);
      active.source = source;
      selectedParser = this.parsers.get(book.format);
      active.parser = selectedParser;

      const sink = new CoordinatorSink(
        this.ipc,
        book.id,
        book.format,
        active.controller.signal,
        onProgress,
      );
      await selectedParser.parse(
        {
          bookId: book.id,
          format: book.format,
          source,
          signal: active.controller.signal,
        },
        sink,
      );
      assertNotAborted(active.controller.signal);
      await sink.assertComplete();
      stage = 'indexing';
      const ready = await this.ipc.finalizeImport(book.id, onProgress);
      assertNotAborted(active.controller.signal);
      return ready;
    } catch (error) {
      if (isCancellation(error, active.controller.signal)) {
        active.controller.abort();
        await this.cancelRustImport(book.id);
        throw cancelledError();
      }

      const userError = toUserError(error);
      try {
        await this.ipc.markImportFailed(book.id, stage, userError.code);
      } catch {
        // Preserve the normalized operation error; Rust logs its own IPC failure.
      }
      throw userError;
    } finally {
      active.source = undefined;
      active.parser = undefined;
      this.#activeImports.delete(book.id);
    }
  }

  async cancel(bookId: string): Promise<void> {
    this.#activeImports.get(bookId)?.controller.abort();
    await this.cancelRustImport(bookId);
  }

  private async cancelRustImport(bookId: string): Promise<void> {
    try {
      await this.ipc.cancelImport(bookId);
    } catch (error) {
      const normalized = toUserError(error);
      if (normalized.code !== 'NOT_FOUND') throw normalized;
    }
  }
}

class CoordinatorSink implements ParserSink {
  #beginCalled = false;
  #begun = false;
  #appended = false;
  #derivedWritten = false;
  #expectedSectionOrdinal = 0;
  #appendChain: Promise<void> = Promise.resolve();
  #derivedWrite: Promise<void> = Promise.resolve();

  constructor(
    private readonly ipc: ImportIpc,
    private readonly bookId: string,
    private readonly format: BookFormat,
    private readonly signal: AbortSignal,
    private readonly onProgress: (event: ImportEvent) => void,
  ) {}

  async begin(metadata: ParsedBookMetadata): Promise<void> {
    assertNotAborted(this.signal);
    if (this.#beginCalled || metadata.title.trim().length === 0) {
      throw contractError('INVALID_INPUT');
    }
    this.#beginCalled = true;
    await this.ipc.beginParse(this.bookId, metadata);
    assertNotAborted(this.signal);
    this.#begun = true;
  }

  async append(sections: NormalizedSectionInput[]): Promise<void> {
    assertNotAborted(this.signal);
    if (!this.#begun || sections.length === 0) {
      throw contractError('INVALID_INPUT');
    }
    for (const section of sections) this.validateSection(section);
    this.#appended = true;
    const batches = batchSections(sections);
    this.#appendChain = this.#appendChain.then(async () => {
      await this.#derivedWrite;
      for (const batch of batches) {
        assertNotAborted(this.signal);
        await this.ipc.appendParsedSections(this.bookId, batch);
        assertNotAborted(this.signal);
      }
    });
    await this.#appendChain;
  }

  progress(event: ImportEvent): void {
    assertNotAborted(this.signal);
    validateProgressEvent(event);
    this.onProgress(event);
  }

  async writeDerivedText(
    name: 'document.html',
    content: string,
  ): Promise<void> {
    assertNotAborted(this.signal);
    if (
      !this.#begun ||
      this.#appended ||
      this.#derivedWritten ||
      content.length === 0
    ) {
      throw contractError('INVALID_INPUT');
    }
    this.#derivedWritten = true;
    this.#derivedWrite = (async () => {
      await this.ipc.writeDerivedText(this.bookId, name, content);
      assertNotAborted(this.signal);
    })();
    await this.#derivedWrite;
  }

  async assertComplete(): Promise<void> {
    assertNotAborted(this.signal);
    await this.#derivedWrite;
    await this.#appendChain;
    if (!this.#begun || !this.#appended) {
      throw contractError('INVALID_INPUT');
    }
    assertNotAborted(this.signal);
  }

  private validateSection(section: NormalizedSectionInput): void {
    if (
      section.ordinal !== this.#expectedSectionOrdinal ||
      section.id !== stableSectionId(this.bookId, section.ordinal) ||
      section.locator.format !== this.format ||
      section.blocks.length === 0 ||
      section.blocks.length > MAX_BATCH_BLOCKS
    ) {
      throw contractError('INVALID_INPUT');
    }
    for (const [blockOrdinal, block] of section.blocks.entries()) {
      if (
        block.ordinal !== blockOrdinal ||
        block.id !==
          stableBlockId(this.bookId, section.ordinal, blockOrdinal) ||
        block.locator.format !== this.format ||
        block.plainText.trim().length === 0
      ) {
        throw contractError('INVALID_INPUT');
      }
    }
    this.#expectedSectionOrdinal += 1;
  }
}

function batchSections(
  sections: readonly NormalizedSectionInput[],
): NormalizedSectionInput[][] {
  const batches: NormalizedSectionInput[][] = [];
  let current: NormalizedSectionInput[] = [];
  let blockCount = 0;

  for (const section of sections) {
    const exceedsLimit =
      current.length === MAX_BATCH_SECTIONS ||
      blockCount + section.blocks.length > MAX_BATCH_BLOCKS;
    if (exceedsLimit && current.length > 0) {
      batches.push(current);
      current = [];
      blockCount = 0;
    }
    current.push(section);
    blockCount += section.blocks.length;
  }
  if (current.length > 0) batches.push(current);
  return batches;
}

function assertNotAborted(signal: AbortSignal): void {
  if (signal.aborted) throw new DOMException('Import cancelled', 'AbortError');
}

function isCancellation(error: unknown, signal: AbortSignal): boolean {
  return (
    signal.aborted ||
    (error instanceof DOMException && error.name === 'AbortError') ||
    (typeof error === 'object' &&
      error !== null &&
      'code' in error &&
      error.code === 'IMPORT_CANCELLED')
  );
}

function validateProgressEvent(event: ImportEvent): void {
  const keys = Object.keys(event).sort();
  if (
    keys.join(',') !== 'completed,messageKey,stage,total' ||
    !['copying', 'parsing', 'indexing'].includes(event.stage) ||
    !Number.isFinite(event.completed) ||
    event.completed < 0 ||
    !Number.isFinite(event.total) ||
    event.total < 0 ||
    event.messageKey.length === 0
  ) {
    throw contractError('INVALID_INPUT');
  }
}

function cancelledError(): UserFacingError {
  return {
    code: 'IMPORT_CANCELLED',
    message: '导入已取消。',
    nextStep: '重新开始导入。',
    diagnosticId: null,
  };
}

function contractError(
  code: 'INVALID_INPUT' | 'REQUEST_CONFLICT',
): UserFacingError {
  return {
    code,
    message:
      code === 'INVALID_INPUT' ? '解析器输出无效。' : '该导入正在处理中。',
    nextStep:
      code === 'INVALID_INPUT' ? '重新导入该教材。' : '先取消当前导入。',
    diagnosticId: null,
  };
}
