import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

import { describe, expect, it, vi } from 'vitest';

import { stableBlockId, stableSectionId } from '../id';
import type { ParserSink } from '../parser-contract';

const bookId = 'e7c92b9f-12ab-4a0d-a5b1-39d1f8fdcf75';

describe('PdfParser', () => {
  it('normalizes the selectable two-page fixture with deterministic page locators', async () => {
    installDomMatrixStub();
    const { PdfParser } = await import('./pdf-parser');
    const { GlobalWorkerOptions } =
      await import('pdfjs-dist/legacy/build/pdf.mjs');
    expect(GlobalWorkerOptions.workerSrc).toContain(
      'pdfjs-dist/legacy/build/pdf.worker.mjs',
    );
    // Vitest runs the browser bundle under Node, where Vite's emitted URL is
    // not a loadable module URL. Keep the production assertion above, then
    // use the equivalent local worker only for this Node-hosted parser run.
    GlobalWorkerOptions.workerSrc = pathToFileURL(
      resolve('node_modules/pdfjs-dist/legacy/build/pdf.worker.mjs'),
    ).href;
    const sink = createSink();
    const source = await readFile('fixtures/textbook.pdf');

    await new PdfParser().parse(
      {
        bookId,
        format: 'pdf',
        source: source.buffer.slice(
          source.byteOffset,
          source.byteOffset + source.byteLength,
        ),
        signal: new AbortController().signal,
      },
      sink,
    );

    expect(sink.metadata).toMatchObject({
      title: expect.stringContaining('TextbookLens'),
      author: 'TextbookLens Contributors',
    });
    expect(sink.sections).toHaveLength(2);
    expect(sink.sections.map((section) => section.id)).toEqual([
      stableSectionId(bookId, 0),
      stableSectionId(bookId, 1),
    ]);
    expect(
      sink.sections
        .flatMap((section) => section.blocks)
        .every((block) => block.plainText.length > 0),
    ).toBe(true);
    expect(
      sink.sections
        .flatMap((section) => section.blocks)
        .some((block) => block.plainText.includes('linear relation')),
    ).toBe(true);
    expect(
      sink.sections
        .flatMap((section) => section.blocks)
        .some((block) => block.plainText.includes('E = mc²')),
    ).toBe(true);
    expect(sink.sections[0]?.locator).toMatchObject({
      format: 'pdf',
      startPage: 1,
      endPage: 1,
      rectsByPage: null,
    });
    expect(
      sink.sections[1]?.blocks.some(
        (block) =>
          block.kind === 'equation' && block.plainText.includes('E = mc²'),
      ),
    ).toBe(true);
    expect(sink.sections[0]?.blocks[0]?.id).toBe(stableBlockId(bookId, 0, 0));
  });

  it('keeps an empty middle page addressable with page-stable IDs', async () => {
    const pageCleanup = vi.fn();
    const documentCleanup = vi.fn();
    const destroy = vi.fn();
    const getPage = vi.fn(async (pageNumber: number) => ({
      getTextContent: async () => ({
        items: [{ str: pageNumber === 2 ? 'empty' : `page-${pageNumber}` }],
      }),
      cleanup: pageCleanup,
    }));
    vi.resetModules();
    vi.doMock('pdfjs-dist/legacy/build/pdf.mjs', () => ({
      getDocument: vi.fn(() => ({
        promise: Promise.resolve({
          numPages: 3,
          getMetadata: async () => ({ info: { Title: 'Synthetic PDF' } }),
          getPage,
          cleanup: documentCleanup,
        }),
        destroy,
      })),
      GlobalWorkerOptions: {},
    }));
    vi.doMock('pdfjs-dist/legacy/build/pdf.worker.mjs?url', () => ({
      default: '/assets/pdf.worker.mjs',
    }));
    vi.doMock('./pdf-layout', async (importOriginal) => {
      const actual = await importOriginal<typeof import('./pdf-layout')>();
      return {
        ...actual,
        pdfTextItems: (items: Array<{ str: string }>) => items,
        normalizePdfPage: (items: Array<{ str: string }>) =>
          items[0]?.str === 'empty' ? [] : [{ text: items[0]?.str ?? '' }],
      };
    });

    try {
      const { PdfParser } = await import('./pdf-parser');
      const sink = createSink();

      await new PdfParser().parse(
        {
          bookId,
          format: 'pdf',
          source: new Uint8Array([37, 80, 68, 70]).buffer,
          signal: new AbortController().signal,
        },
        sink,
      );

      expect(sink.sections).toHaveLength(3);
      expect(sink.sections.map((section) => section.ordinal)).toEqual([
        0, 1, 2,
      ]);
      expect(sink.sections.map((section) => section.id)).toEqual([
        stableSectionId(bookId, 0),
        stableSectionId(bookId, 1),
        stableSectionId(bookId, 2),
      ]);
      expect(
        sink.sections.map((section) => {
          expect(section.locator.format).toBe('pdf');
          return section.locator.format === 'pdf'
            ? section.locator.startPage
            : null;
        }),
      ).toEqual([1, 2, 3]);
      expect(sink.sections.map((section) => section.blocks[0]?.id)).toEqual([
        stableBlockId(bookId, 0, 0),
        undefined,
        stableBlockId(bookId, 2, 0),
      ]);
      expect(sink.progress).toHaveBeenCalledTimes(3);
      expect(getPage).toHaveBeenCalledTimes(3);
      expect(pageCleanup).toHaveBeenCalledTimes(3);
      expect(documentCleanup).toHaveBeenCalledOnce();
      expect(destroy).toHaveBeenCalledOnce();
    } finally {
      vi.doUnmock('pdfjs-dist/legacy/build/pdf.mjs');
      vi.doUnmock('pdfjs-dist/legacy/build/pdf.worker.mjs?url');
      vi.doUnmock('./pdf-layout');
      vi.resetModules();
    }
  });

  it('imports a fully image-only PDF as addressable pages', async () => {
    const cleanup = vi.fn();
    const destroy = vi.fn();
    vi.resetModules();
    vi.doMock('pdfjs-dist/legacy/build/pdf.mjs', () => ({
      getDocument: vi.fn(() => ({
        promise: Promise.resolve({
          numPages: 2,
          getMetadata: async () => ({ info: {} }),
          getPage: async () => ({
            getTextContent: async () => ({ items: [] }),
            cleanup,
          }),
          cleanup,
        }),
        destroy,
      })),
      GlobalWorkerOptions: {},
    }));
    vi.doMock('pdfjs-dist/legacy/build/pdf.worker.mjs?url', () => ({
      default: '/assets/pdf.worker.mjs',
    }));

    try {
      const { PdfParser } = await import('./pdf-parser');
      const sink = createSink();
      await new PdfParser().parse(
        {
          bookId,
          format: 'pdf',
          source: new Uint8Array([37, 80, 68, 70]).buffer,
          signal: new AbortController().signal,
        },
        sink,
      );

      expect(sink.sections).toHaveLength(2);
      expect(
        sink.sections.every((section) => section.blocks.length === 0),
      ).toBe(true);
      expect(sink.sections.map((section) => section.id)).toEqual([
        stableSectionId(bookId, 0),
        stableSectionId(bookId, 1),
      ]);
      expect(destroy).toHaveBeenCalledOnce();
    } finally {
      vi.doUnmock('pdfjs-dist/legacy/build/pdf.mjs');
      vi.doUnmock('pdfjs-dist/legacy/build/pdf.worker.mjs?url');
      vi.resetModules();
    }
  });

  it('maps PDF.js corruption and encryption errors to distinct safe codes', async () => {
    const destroy = vi.fn();
    vi.resetModules();
    vi.doMock('pdfjs-dist/legacy/build/pdf.mjs', () => ({
      getDocument: ({ data }: { data: Uint8Array }) => ({
        promise: Promise.reject(
          new Error(data[0] === 1 ? 'Password required' : 'Invalid PDF'),
        ),
        destroy,
      }),
      GlobalWorkerOptions: {},
    }));
    vi.doMock('pdfjs-dist/legacy/build/pdf.worker.mjs?url', () => ({
      default: '/assets/pdf.worker.mjs',
    }));

    try {
      const { PdfParser } = await import('./pdf-parser');
      const parser = new PdfParser();

      await expect(
        parser.parse(
          {
            bookId,
            format: 'pdf',
            source: new Uint8Array([0]).buffer,
            signal: new AbortController().signal,
          },
          createSink(),
        ),
      ).rejects.toMatchObject({
        code: 'FILE_CORRUPTED',
        diagnosticId: null,
      });
      await expect(
        parser.parse(
          {
            bookId,
            format: 'pdf',
            source: new Uint8Array([1]).buffer,
            signal: new AbortController().signal,
          },
          createSink(),
        ),
      ).rejects.toMatchObject({
        code: 'FILE_ENCRYPTED_OR_DRM',
        diagnosticId: null,
      });
      expect(destroy).toHaveBeenCalledTimes(2);
    } finally {
      vi.doUnmock('pdfjs-dist/legacy/build/pdf.mjs');
      vi.doUnmock('pdfjs-dist/legacy/build/pdf.worker.mjs?url');
      vi.resetModules();
    }
  });
});

function createSink(): ParserSink & {
  metadata: unknown;
  sections: Parameters<ParserSink['append']>[0];
} {
  return {
    metadata: undefined,
    sections: [],
    begin: vi.fn(async function (this: { metadata: unknown }, metadata) {
      this.metadata = metadata;
    }),
    append: vi.fn(async function (
      this: { sections: Parameters<ParserSink['append']>[0] },
      sections,
    ) {
      this.sections.push(...sections);
    }),
    progress: vi.fn(),
    writeDerivedText: vi.fn(),
  };
}

function installDomMatrixStub(): void {
  if ('DOMMatrix' in globalThis) return;
  class DomMatrixStub {}
  Object.assign(globalThis, { DOMMatrix: DomMatrixStub });
}
