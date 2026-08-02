import { afterEach, describe, expect, it, vi } from 'vitest';

import type { BookSummary } from '../../lib/generated/book';
import type { NormalizedSectionInput } from '../../lib/generated/document';
import { TauriImportIpc, type ImportIpc } from '../../lib/ipc';
import { clearMocks, installTauriMock } from '../../test/tauri-mock';
import { stableBlockId, stableSectionId } from './id';
import { ImportCoordinator } from './ImportCoordinator';
import type {
  BeginImportOutcome,
  DocumentParser,
  ImportEvent,
  ImportStage,
  ParsedBookMetadata,
} from './parser-contract';
import { ParserRegistry } from './parser-registry';

const BOOK_ID = '4f9a2c86-0da8-4dd4-a255-39b4cff89c66';

afterEach(() => clearMocks());

function book(overrides: Partial<BookSummary> = {}): BookSummary {
  return {
    id: BOOK_ID,
    title: 'Fixture',
    author: null,
    language: null,
    format: 'pdf',
    importStatus: 'parsing',
    importErrorCode: null,
    importErrorMessage: null,
    importErrorStage: null,
    readingProgress: 0,
    createdAt: '2026-08-02T00:00:00Z',
    updatedAt: '2026-08-02T00:00:00Z',
    lastOpenedAt: null,
    ...overrides,
  };
}

function metadata(): ParsedBookMetadata {
  return { title: 'Fixture', author: null, language: 'zh-CN' };
}

function section(ordinal: number, blockCount = 1): NormalizedSectionInput {
  return {
    id: stableSectionId(BOOK_ID, ordinal),
    parentId: null,
    ordinal,
    title: `Section ${ordinal}`,
    locator: {
      format: 'pdf',
      startPage: ordinal + 1,
      endPage: ordinal + 1,
      rectsByPage: null,
    },
    blocks: Array.from({ length: blockCount }, (_, blockOrdinal) => ({
      id: stableBlockId(BOOK_ID, ordinal, blockOrdinal),
      ordinal: blockOrdinal,
      kind: 'paragraph' as const,
      plainText: `Block ${ordinal}:${blockOrdinal}`,
      locator: {
        format: 'pdf' as const,
        startPage: ordinal + 1,
        endPage: ordinal + 1,
        rectsByPage: null,
      },
    })),
  };
}

function docxSection(ordinal: number): NormalizedSectionInput {
  const blockId = stableBlockId(BOOK_ID, ordinal, 0);
  return {
    id: stableSectionId(BOOK_ID, ordinal),
    parentId: null,
    ordinal,
    title: `Section ${ordinal}`,
    locator: {
      format: 'docx',
      startBlockId: blockId,
      startOffset: 0,
      endBlockId: blockId,
      endOffset: 5,
    },
    blocks: [
      {
        id: blockId,
        ordinal: 0,
        kind: 'paragraph',
        plainText: 'Block',
        locator: {
          format: 'docx',
          startBlockId: blockId,
          startOffset: 0,
          endBlockId: blockId,
          endOffset: 5,
        },
      },
    ],
  };
}

class FakeImportIpc implements ImportIpc {
  readonly calls: string[] = [];
  readonly batches: NormalizedSectionInput[][] = [];
  outcome: BeginImportOutcome = { outcome: 'created', book: book() };
  binary: Uint8Array | ArrayBuffer = new Uint8Array([1, 2, 3]);
  appendError: unknown;
  appendGate: Promise<void> | undefined;
  onCancel: (() => void) | undefined;

  async beginImport(
    _sourcePath: string,
    _onProgress: (event: ImportEvent) => void,
  ): Promise<BeginImportOutcome> {
    void _sourcePath;
    void _onProgress;
    this.calls.push('begin_import');
    return this.outcome;
  }

  async readBookSource(_bookId: string): Promise<Uint8Array | ArrayBuffer> {
    void _bookId;
    this.calls.push('read_book_source');
    return this.binary;
  }

  async beginParse(
    _bookId: string,
    _metadata: ParsedBookMetadata,
  ): Promise<void> {
    void _bookId;
    void _metadata;
    this.calls.push('begin_parse');
  }

  async appendParsedSections(
    _bookId: string,
    sections: NormalizedSectionInput[],
  ): Promise<void> {
    void _bookId;
    this.calls.push('append_parsed_sections');
    this.batches.push(sections);
    if (this.appendGate) await this.appendGate;
    if (this.appendError) throw this.appendError;
  }

  async writeDerivedText(
    _bookId: string,
    _name: 'document.html',
    _content: string,
  ): Promise<void> {
    void _bookId;
    void _name;
    void _content;
    this.calls.push('write_derived_text');
  }

  async finalizeImport(
    _bookId: string,
    _onProgress: (event: ImportEvent) => void,
  ): Promise<BookSummary> {
    void _bookId;
    void _onProgress;
    this.calls.push('finalize_import');
    return book({ importStatus: 'ready' });
  }

  async cancelImport(_bookId: string): Promise<void> {
    void _bookId;
    this.calls.push('cancel_import');
    this.onCancel?.();
  }

  async markImportFailed(
    _bookId: string,
    _stage: ImportStage,
    _code: string,
  ): Promise<void> {
    void _bookId;
    void _stage;
    void _code;
    this.calls.push('mark_import_failed');
  }
}

function parser(
  parse: DocumentParser['parse'],
  format: DocumentParser['format'] = 'pdf',
): DocumentParser {
  return { format, parse };
}

function coordinator(ipc: FakeImportIpc, documentParser: DocumentParser) {
  return new ImportCoordinator(ipc, new ParserRegistry([documentParser]));
}

describe('ImportCoordinator', () => {
  it('sequences IPC, preserves binary view bounds, and enforces both batch limits', async () => {
    const ipc = new FakeImportIpc();
    const backing = new Uint8Array([99, 1, 2, 3, 88]);
    ipc.binary = backing.subarray(1, 4);
    const parse = vi.fn<DocumentParser['parse']>(async (context, sink) => {
      expect(Array.from(new Uint8Array(context.source))).toEqual([1, 2, 3]);
      expect(context.source.byteLength).toBe(3);
      await sink.begin(metadata());
      await sink.append(
        Array.from({ length: 30 }, (_, ordinal) => section(ordinal, 20)),
      );
    });

    const result = await coordinator(ipc, parser(parse)).importDocument(
      'C:/transient.pdf',
    );

    expect(result.importStatus).toBe('ready');
    expect(parse).toHaveBeenCalledOnce();
    expect(ipc.calls).toEqual([
      'begin_import',
      'read_book_source',
      'begin_parse',
      'append_parsed_sections',
      'append_parsed_sections',
      'finalize_import',
    ]);
    expect(ipc.batches.map((batch) => batch.length)).toEqual([25, 5]);
    expect(
      ipc.batches.map((batch) =>
        batch.reduce((total, item) => total + item.blocks.length, 0),
      ),
    ).toEqual([500, 100]);
  });

  it('returns duplicates without reading bytes or starting a parser', async () => {
    const ipc = new FakeImportIpc();
    ipc.outcome = {
      outcome: 'duplicate',
      book: book({ importStatus: 'ready' }),
    };
    const parse = vi.fn<DocumentParser['parse']>();

    const result = await coordinator(ipc, parser(parse)).importDocument(
      'C:/duplicate.pdf',
    );

    expect(result.id).toBe(BOOK_ID);
    expect(parse).not.toHaveBeenCalled();
    expect(ipc.calls).toEqual(['begin_import']);
  });

  it('owns duplicate registration and unsupported-format errors without placeholders', () => {
    const pdf = parser(vi.fn<DocumentParser['parse']>());
    expect(() => new ParserRegistry([pdf, pdf])).toThrowError(
      expect.objectContaining({ code: 'REQUEST_CONFLICT' }),
    );
    const registry = new ParserRegistry([pdf]);
    expect(() => registry.get('epub')).toThrowError(
      expect.objectContaining({ code: 'UNSUPPORTED_FILE_TYPE' }),
    );
    expect(() => registry.get('future-format')).toThrowError(
      expect.objectContaining({ code: 'UNSUPPORTED_FILE_TYPE' }),
    );
  });

  it('validates tagged IPC outcomes and sends only normalized failure fields', async () => {
    const calls = installTauriMock((command) => {
      if (command === 'begin_import') {
        return { outcome: 'created', book: book() };
      }
      return undefined;
    });
    const ipc = new TauriImportIpc();

    await expect(
      ipc.beginImport('C:/only-transient.pdf', vi.fn()),
    ).resolves.toMatchObject({
      outcome: 'created',
    });
    await ipc.markImportFailed(BOOK_ID, 'parsing', 'FILE_CORRUPTED');

    expect(calls[0]?.payload).toMatchObject({
      request: { sourcePath: 'C:/only-transient.pdf' },
    });
    expect(calls[1]).toEqual({
      command: 'mark_import_failed',
      payload: {
        bookId: BOOK_ID,
        stage: 'parsing',
        code: 'FILE_CORRUPTED',
      },
    });
  });

  it.each([
    ['missing begin', async () => {}],
    [
      'begin without appended content',
      async (
        _context: Parameters<DocumentParser['parse']>[0],
        sink: Parameters<DocumentParser['parse']>[1],
      ) => {
        await sink.begin(metadata());
      },
    ],
    [
      'empty metadata',
      async (
        _context: Parameters<DocumentParser['parse']>[0],
        sink: Parameters<DocumentParser['parse']>[1],
      ) => {
        await sink.begin({ title: '   ', author: null, language: null });
      },
    ],
    [
      'duplicate begin',
      async (
        _context: Parameters<DocumentParser['parse']>[0],
        sink: Parameters<DocumentParser['parse']>[1],
      ) => {
        await sink.begin(metadata());
        await sink.begin(metadata());
      },
    ],
    [
      'append before begin',
      async (
        _context: Parameters<DocumentParser['parse']>[0],
        sink: Parameters<DocumentParser['parse']>[1],
      ) => {
        await sink.append([section(0)]);
      },
    ],
    [
      'empty append',
      async (
        _context: Parameters<DocumentParser['parse']>[0],
        sink: Parameters<DocumentParser['parse']>[1],
      ) => {
        await sink.begin(metadata());
        await sink.append([]);
      },
    ],
    [
      'derived write after append',
      async (
        _context: Parameters<DocumentParser['parse']>[0],
        sink: Parameters<DocumentParser['parse']>[1],
      ) => {
        await sink.begin(metadata());
        await sink.append([section(0)]);
        await sink.writeDerivedText('document.html', '<p>late</p>');
      },
    ],
    [
      'one oversized section',
      async (
        _context: Parameters<DocumentParser['parse']>[0],
        sink: Parameters<DocumentParser['parse']>[1],
      ) => {
        await sink.begin(metadata());
        await sink.append([section(0, 501)]);
      },
    ],
    [
      'random parser IDs',
      async (
        _context: Parameters<DocumentParser['parse']>[0],
        sink: Parameters<DocumentParser['parse']>[1],
      ) => {
        const invalid = section(0);
        invalid.id = 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa';
        await sink.begin(metadata());
        await sink.append([invalid]);
      },
    ],
    [
      'progress carrying extra data',
      async (
        _context: Parameters<DocumentParser['parse']>[0],
        sink: Parameters<DocumentParser['parse']>[1],
      ) => {
        await sink.begin(metadata());
        sink.progress({
          stage: 'parsing',
          completed: 1,
          total: 2,
          messageKey: 'import.parsing',
          sourcePath: 'must-not-cross-contract',
        } as ImportEvent);
      },
    ],
  ])('rejects %s and never finalizes', async (_name, parse) => {
    const ipc = new FakeImportIpc();

    await expect(
      coordinator(ipc, parser(parse)).importDocument('C:/bad.pdf'),
    ).rejects.toBeDefined();

    expect(ipc.calls).toContain('mark_import_failed');
    expect(ipc.calls).not.toContain('finalize_import');
  });

  it('requires exactly one DOCX derived write between begin and the first append', async () => {
    const ipc = new FakeImportIpc();
    ipc.outcome = {
      outcome: 'created',
      book: book({ format: 'docx' }),
    };
    const parse = parser(async (_context, sink) => {
      await sink.begin(metadata());
      await sink.writeDerivedText('document.html', '<p>safe</p>');
      await sink.append([docxSection(0)]);
    }, 'docx');

    await coordinator(ipc, parse).importDocument('C:/derived.pdf');

    expect(ipc.calls.indexOf('begin_parse')).toBeLessThan(
      ipc.calls.indexOf('write_derived_text'),
    );
    expect(ipc.calls.indexOf('write_derived_text')).toBeLessThan(
      ipc.calls.indexOf('append_parsed_sections'),
    );
  });

  it.each([
    [
      'missing',
      async (
        _context: Parameters<DocumentParser['parse']>[0],
        sink: Parameters<DocumentParser['parse']>[1],
      ) => {
        await sink.begin(metadata());
        await sink.append([docxSection(0)]);
      },
    ],
    [
      'duplicate',
      async (
        _context: Parameters<DocumentParser['parse']>[0],
        sink: Parameters<DocumentParser['parse']>[1],
      ) => {
        await sink.begin(metadata());
        await sink.writeDerivedText('document.html', '<p>safe</p>');
        await sink.writeDerivedText('document.html', '<p>duplicate</p>');
        await sink.append([docxSection(0)]);
      },
    ],
    [
      'late',
      async (
        _context: Parameters<DocumentParser['parse']>[0],
        sink: Parameters<DocumentParser['parse']>[1],
      ) => {
        await sink.begin(metadata());
        await sink.writeDerivedText('document.html', '<p>safe</p>');
        await sink.append([docxSection(0)]);
        await sink.writeDerivedText('document.html', '<p>late</p>');
      },
    ],
  ])(
    'rejects a %s DOCX derived write contract and never finalizes',
    async (_name, parse) => {
      const ipc = new FakeImportIpc();
      ipc.outcome = {
        outcome: 'created',
        book: book({ format: 'docx' }),
      };

      await expect(
        coordinator(ipc, parser(parse, 'docx')).importDocument('C:/bad.docx'),
      ).rejects.toBeDefined();

      expect(ipc.calls).toContain('mark_import_failed');
      expect(ipc.calls).not.toContain('finalize_import');
    },
  );

  it.each(['pdf', 'epub'] as const)(
    'does not require derived HTML for %s',
    async (format) => {
      const ipc = new FakeImportIpc();
      ipc.outcome = { outcome: 'created', book: book({ format }) };
      const formatSection: NormalizedSectionInput = {
        ...section(0),
        locator:
          format === 'pdf'
            ? section(0).locator
            : {
                format: 'epub',
                cfi: 'epubcfi(/6/2!/4/2)',
                sectionId: stableSectionId(BOOK_ID, 0),
              },
        blocks: section(0).blocks.map((block) => ({
          ...block,
          locator:
            format === 'pdf'
              ? block.locator
              : {
                  format: 'epub' as const,
                  cfi: 'epubcfi(/6/2!/4/2)',
                  sectionId: stableSectionId(BOOK_ID, 0),
                },
        })),
      };

      await coordinator(
        ipc,
        parser(async (_context, sink) => {
          await sink.begin(metadata());
          await sink.append([formatSection]);
        }, format),
      ).importDocument(`C:/book.${format}`);

      expect(ipc.calls).toContain('finalize_import');
      expect(ipc.calls).not.toContain('write_derived_text');
    },
  );

  it('marks ordinary parser and append IPC failures without finalizing', async () => {
    const parserIpc = new FakeImportIpc();
    await expect(
      coordinator(
        parserIpc,
        parser(async (_context, sink) => {
          await sink.begin(metadata());
          throw new Error('private parser detail');
        }),
      ).importDocument('C:/parser-error.pdf'),
    ).rejects.toMatchObject({ code: 'DATABASE_ERROR' });
    expect(parserIpc.calls).toContain('mark_import_failed');
    expect(parserIpc.calls).not.toContain('finalize_import');

    const appendIpc = new FakeImportIpc();
    appendIpc.appendError = {
      code: 'DATABASE_ERROR',
      message: 'safe',
      nextStep: 'retry',
      diagnosticId: null,
    };
    await expect(
      coordinator(
        appendIpc,
        parser(async (_context, sink) => {
          await sink.begin(metadata());
          await sink.append([section(0)]);
        }),
      ).importDocument('C:/append-error.pdf'),
    ).rejects.toMatchObject({ code: 'DATABASE_ERROR' });
    expect(appendIpc.calls).toContain('mark_import_failed');
    expect(appendIpc.calls).not.toContain('finalize_import');
  });

  it('maps AbortError to idempotent cancellation and never marks or finalizes', async () => {
    const ipc = new FakeImportIpc();
    await expect(
      coordinator(
        ipc,
        parser(async (_context, sink) => {
          await sink.begin(metadata());
          throw new DOMException('cancelled', 'AbortError');
        }),
      ).importDocument('C:/cancel.pdf'),
    ).rejects.toMatchObject({ code: 'IMPORT_CANCELLED' });

    expect(ipc.calls).toContain('cancel_import');
    expect(ipc.calls).not.toContain('mark_import_failed');
    expect(ipc.calls).not.toContain('finalize_import');
  });

  it('aborts JS ownership before Rust cancellation and rechecks after awaited append', async () => {
    const ipc = new FakeImportIpc();
    let releaseAppend!: () => void;
    ipc.appendGate = new Promise<void>((resolve) => {
      releaseAppend = resolve;
    });
    let parserSignal: AbortSignal | undefined;
    let appendStarted!: () => void;
    const started = new Promise<void>((resolve) => {
      appendStarted = resolve;
    });
    const activeCoordinator = coordinator(
      ipc,
      parser(async (context, sink) => {
        parserSignal = context.signal;
        await sink.begin(metadata());
        appendStarted();
        await sink.append([section(0)]);
      }),
    );
    ipc.onCancel = () => expect(parserSignal?.aborted).toBe(true);

    const operation = activeCoordinator.importDocument('C:/cancel-race.pdf');
    await started;
    const cancellation = activeCoordinator.cancel(BOOK_ID);
    releaseAppend();
    await cancellation;

    await expect(operation).rejects.toMatchObject({ code: 'IMPORT_CANCELLED' });
    expect(ipc.calls).not.toContain('finalize_import');
    expect(activeCoordinator.activeJobCount).toBe(0);
  });
});
