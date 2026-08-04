import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

import { describe, expect, it } from 'vitest';

import { assessPdfPageQuality } from './pdf-quality';

describe('assessPdfPageQuality', () => {
  it.each([
    ['no_text', []],
    [
      'very_low_text_coverage',
      [item('Readable words but too little page coverage', 20, 10)],
    ],
    [
      'high_replacement_or_control_ratio',
      [
        item(
          `${'\ufffd'.repeat(8)} readable extracted textbook words for coverage`,
          500,
          20,
        ),
      ],
    ],
    ['extreme_duplicate_glyphs', [item('A'.repeat(100), 500, 20)]],
  ] as const)('returns the versioned %s reason', (qualityReason, items) => {
    expect(
      assessPdfPageQuality({ pageNumber: 1, width: 600, height: 842, items }),
    ).toEqual({
      schemaVersion: 1,
      pageNumber: 1,
      qualityReason,
      status: 'needs_review',
    });
  });

  it('detects contradictory overlapping text geometry without changing reading order', () => {
    const result = assessPdfPageQuality({
      pageNumber: 3,
      width: 600,
      height: 842,
      items: [
        item('First extracted statement has enough text.', 400, 100),
        item('Second conflicting statement has enough text.', 400, 100),
      ],
    });

    expect(result.qualityReason).toBe('layout_contradiction');
    expect(result.status).toBe('needs_review');
  });

  it('marks reliable local text as not required', () => {
    expect(
      assessPdfPageQuality({
        pageNumber: 2,
        width: 600,
        height: 842,
        items: [
          item(
            'This selectable textbook paragraph contains enough distinct meaningful characters to stay in the local text path.',
            500,
            20,
          ),
        ],
      }),
    ).toMatchObject({ qualityReason: 'reliable_text', status: 'not_required' });
  });

  it('classifies the self-made scanned and mixed local fixtures without network access', async () => {
    installDomMatrixStub();
    const { inspectLocalPdfPageQuality } =
      await import('../import/parsers/pdf-parser');
    const { GlobalWorkerOptions } =
      await import('pdfjs-dist/legacy/build/pdf.mjs');
    GlobalWorkerOptions.workerSrc = pathToFileURL(
      resolve('node_modules/pdfjs-dist/legacy/build/pdf.worker.mjs'),
    ).href;
    const scanned = await readFile('fixtures/source/scanned-textbook.pdf');
    const mixed = await readFile('fixtures/source/mixed-quality-textbook.pdf');

    await expect(
      inspectLocalPdfPageQuality(
        scanned.buffer.slice(
          scanned.byteOffset,
          scanned.byteOffset + scanned.byteLength,
        ),
        new AbortController().signal,
      ),
    ).resolves.toEqual([
      expect.objectContaining({
        qualityReason: 'no_text',
        status: 'needs_review',
      }),
    ]);
    await expect(
      inspectLocalPdfPageQuality(
        mixed.buffer.slice(
          mixed.byteOffset,
          mixed.byteOffset + mixed.byteLength,
        ),
        new AbortController().signal,
      ),
    ).resolves.toEqual([
      expect.objectContaining({
        qualityReason: 'reliable_text',
        status: 'not_required',
      }),
      expect.objectContaining({
        qualityReason: 'no_text',
        status: 'needs_review',
      }),
    ]);
  });
});

function item(str: string, width: number, height: number) {
  return { str, transform: [height, 0, 0, height, 48, 700] as const, width };
}

function installDomMatrixStub(): void {
  if ('DOMMatrix' in globalThis) return;
  class DomMatrixStub {}
  Object.assign(globalThis, { DOMMatrix: DomMatrixStub });
}
