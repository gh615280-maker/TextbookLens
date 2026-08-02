import { readFile } from 'node:fs/promises';

import { Document, ImageRun, Packer, Paragraph } from 'docx';
import { describe, expect, it, vi } from 'vitest';

import { stableBlockId, stableSectionId } from '../id';
import type { ParserSink } from '../parser-contract';
import { DocxParser } from './docx-parser';

const bookId = 'f50b99d9-0c1a-461d-9a2f-0fdc47f2a05b';

describe('DocxParser', () => {
  it('normalizes the fixture and writes sanitized derived HTML once', async () => {
    const source = await readFile('fixtures/textbook.docx');
    const sink = createSink();

    await new DocxParser().parse(
      {
        bookId,
        format: 'docx',
        source: Uint8Array.from(source).buffer,
        signal: new AbortController().signal,
      },
      sink,
    );

    expect(sink.metadata).toMatchObject({ title: expect.any(String) });
    expect(sink.sections.map((section) => section.title)).toEqual(
      expect.arrayContaining([
        expect.stringContaining('第一章'),
        expect.stringContaining('第二章'),
      ]),
    );
    expect(
      sink.sections
        .flatMap((section) => section.blocks)
        .map((block) => block.kind),
    ).toEqual(
      expect.arrayContaining(['list_item', 'table', 'figure_caption', 'code']),
    );
    expect(
      sink.sections
        .flatMap((section) => section.blocks)
        .some((block) => block.plainText.includes('E = mc')),
    ).toBe(true);
    expect(sink.sections[0]?.id).toBe(stableSectionId(bookId, 0));
    expect(sink.sections[0]?.blocks[0]?.id).toBe(stableBlockId(bookId, 0, 0));
    expect(sink.derived).toHaveLength(1);
    expect(sink.derived[0]?.content).not.toMatch(/<script|https:\/\//iu);
  });

  it('returns NO_EXTRACTABLE_TEXT for an all-image DOCX variant', async () => {
    const image = await readFile('fixtures/source/figure-energy.png');
    const source = await Packer.toArrayBuffer(
      new Document({
        sections: [
          {
            children: [
              new Paragraph({
                children: [
                  new ImageRun({
                    type: 'png',
                    data: image,
                    transformation: { width: 1, height: 1 },
                  }),
                ],
              }),
            ],
          },
        ],
      }),
    );

    await expect(
      new DocxParser().parse(
        {
          bookId,
          format: 'docx',
          source,
          signal: new AbortController().signal,
        },
        createSink(),
      ),
    ).rejects.toMatchObject({ code: 'NO_EXTRACTABLE_TEXT' });
  });
});

function createSink(): ParserSink & {
  metadata: unknown;
  sections: Parameters<ParserSink['append']>[0];
  derived: Array<{ name: string; content: string }>;
} {
  return {
    metadata: undefined,
    sections: [],
    derived: [],
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
    writeDerivedText: vi.fn(async function (
      this: { derived: Array<{ name: string; content: string }> },
      name,
      content,
    ) {
      this.derived.push({ name, content });
    }),
  };
}
