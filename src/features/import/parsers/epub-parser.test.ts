import { readFile } from 'node:fs/promises';

import ePub from 'epubjs';
import JSZip from 'jszip';
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
    const sections: Array<{
      document: Document;
      load(request?: unknown): Promise<Document>;
      unload(): void;
      cfiFromElement(element: Element): string;
    }> = [];
    book.spine.each(
      (section: {
        document: Document;
        load(request?: unknown): Promise<Document>;
        unload(): void;
        cfiFromElement(element: Element): string;
      }) => sections.push(section),
    );

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
      {
        bookId,
        format: 'epub',
        source: toArrayBuffer(source),
        signal: new AbortController().signal,
      },
      sink,
    );

    expect(sink.sections).toHaveLength(2);
    expect(
      sink.sections.every((section) => section.locator.format === 'epub'),
    ).toBe(true);
    expect(
      sink.sections
        .flatMap((section) => section.blocks)
        .some((block) => block.plainText.includes('linear relation')),
    ).toBe(true);
    expect(
      sink.sections
        .flatMap((section) => section.blocks)
        .some(
          (block) =>
            block.locator.format === 'epub' &&
            block.locator.cfi.startsWith('epubcfi('),
        ),
    ).toBe(true);
    expect(sink.sections.flatMap((section) => section.blocks)).toEqual(
      expect.arrayContaining([
        expect.objectContaining({
          kind: 'equation',
          plainText: expect.stringContaining('E = mc²'),
        }),
      ]),
    );
  });

  it('imports a synthetic EPUB containing safe XML node boundaries', async () => {
    const source = await createXmlBoundaryEpub();
    const sink = createSink();

    await new EpubParser().parse(
      {
        bookId,
        format: 'epub',
        source,
        signal: new AbortController().signal,
      },
      sink,
    );

    expect(sink.begin).toHaveBeenCalledWith({
      title: 'Synthetic XML boundary EPUB',
      author: 'TextbookLens fixtures',
      language: 'en',
    });
    expect(sink.sections).toHaveLength(1);
    expect(sink.sections[0]?.blocks.map((block) => block.plainText)).toEqual(
      expect.arrayContaining([
        'Synthetic chapter',
        'Normal EPUB text with safe CDATA boundary text.',
      ]),
    );
  });
});

function createSink(): ParserSink & {
  sections: Parameters<ParserSink['append']>[0];
} {
  return {
    sections: [],
    begin: vi.fn(),
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

function toArrayBuffer(bytes: Uint8Array): ArrayBuffer {
  return Uint8Array.from(bytes).buffer;
}

async function createXmlBoundaryEpub(): Promise<ArrayBuffer> {
  const archive = new JSZip();
  archive.file('mimetype', 'application/epub+zip', { compression: 'STORE' });
  archive.file(
    'META-INF/container.xml',
    `<?xml version="1.0" encoding="UTF-8"?>
<?fixture safe?>
<container xmlns="urn:oasis:names:tc:opendocument:xmlns:container" version="1.0">
  <!-- synthetic safe comment -->
  <rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles>
</container>`,
  );
  archive.file(
    'OEBPS/content.opf',
    `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE package>
<?fixture safe?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="book-id">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier id="book-id">synthetic-xml-boundary</dc:identifier>
    <dc:title>Synthetic XML boundary EPUB</dc:title>
    <dc:language>en</dc:language>
    <dc:creator>TextbookLens fixtures</dc:creator>
  </metadata>
  <manifest>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
    <item id="chapter" href="chapter.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine><itemref idref="chapter"/></spine>
</package>`,
  );
  archive.file(
    'OEBPS/nav.xhtml',
    `<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
  <head><title>Contents</title></head>
  <body><nav epub:type="toc"><ol><li><a href="chapter.xhtml">Synthetic chapter</a></li></ol></nav></body>
</html>`,
  );
  archive.file(
    'OEBPS/chapter.xhtml',
    `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE html>
<?fixture safe?>
<html xmlns="http://www.w3.org/1999/xhtml">
  <head><title>Synthetic chapter</title><!-- synthetic safe comment --></head>
  <body><h1>Synthetic chapter</h1><p>Normal EPUB text with safe <![CDATA[CDATA boundary]]> text.</p></body>
</html>`,
  );
  return archive.generateAsync({
    type: 'arraybuffer',
    compression: 'DEFLATE',
    compressionOptions: { level: 9 },
  });
}
