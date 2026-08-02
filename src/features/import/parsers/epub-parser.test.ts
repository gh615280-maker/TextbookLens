import { readFile } from 'node:fs/promises';

import ePub from 'epubjs';
import { describe, expect, it, vi } from 'vitest';

import type { ParserSink } from '../parser-contract';
import { EpubParser } from './epub-parser';

const bookId = 'b1d91c0f-6a6c-4b5e-b10f-c5d5ba0ae670';

describe('EpubParser', () => {
  it('proves EPUB.js CFIs resolve to the source spine item', async () => {
    const source = await readFile('fixtures/textbook.epub');
    const book = ePub({ replacements: 'none' });
    await book.open(toArrayBuffer(source));
    await book.ready;
    const sections: Array<{ document: Document; load(request?: unknown): Promise<Document>; unload(): void; cfiFromElement(element: Element): string }> = [];
    book.spine.each((section: { document: Document; load(request?: unknown): Promise<Document>; unload(): void; cfiFromElement(element: Element): string }) => sections.push(section));

    for (const section of sections) {
      const document = await section.load(book.load.bind(book));
      const element = document.querySelector('h1, p');
      expect(element).not.toBeNull();
      const cfi = section.cfiFromElement(element!);
      const range = await book.getRange(cfi);
      expect(range.startContainer.ownerDocument).toBe(section.document);
      section.unload();
    }
    book.destroy();
  });

  it('normalizes each spine item with resolvable CFI locators', async () => {
    const source = await readFile('fixtures/textbook.epub');
    const sink = createSink();

    await new EpubParser().parse(
      { bookId, format: 'epub', source: toArrayBuffer(source), signal: new AbortController().signal },
      sink,
    );

    expect(sink.sections).toHaveLength(2);
    expect(sink.sections.every((section) => section.locator.format === 'epub')).toBe(true);
    expect(sink.sections.flatMap((section) => section.blocks).some((block) => block.plainText.includes('linear relation'))).toBe(true);
    expect(sink.sections.flatMap((section) => section.blocks).some((block) => block.locator.format === 'epub' && block.locator.cfi.startsWith('epubcfi('))).toBe(true);
  });
});

function createSink(): ParserSink & { sections: Parameters<ParserSink['append']>[0] } {
  return {
    sections: [],
    begin: vi.fn(),
    append: vi.fn(async function (this: { sections: Parameters<ParserSink['append']>[0] }, sections) { this.sections.push(...sections); }),
    progress: vi.fn(),
    writeDerivedText: vi.fn(),
  };
}

function toArrayBuffer(bytes: Uint8Array): ArrayBuffer {
  return Uint8Array.from(bytes).buffer;
}
