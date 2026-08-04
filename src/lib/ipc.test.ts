import { describe, expect, it } from 'vitest';

import { parseBookSummary } from './ipc';

function summary(overrides: Record<string, unknown> = {}) {
  return {
    id: '4f9a2c86-0da8-4dd4-a255-39b4cff89c66',
    title: 'Linear algebra',
    originalFilename: 'linear-algebra.pdf',
    author: null,
    language: 'en',
    format: 'pdf',
    importStatus: 'ready',
    importErrorCode: null,
    importErrorMessage: null,
    importErrorStage: null,
    readingProgress: 0,
    indexAggregate: {
      status: 'not_required',
      totalPages: 0,
      indexedPages: 0,
      reviewPages: 0,
      failedPages: 0,
    },
    createdAt: '2026-08-04T00:00:00.000Z',
    updatedAt: '2026-08-04T00:00:00.000Z',
    lastOpenedAt: null,
    ...overrides,
  };
}

describe('BookSummary IPC validation', () => {
  it.each([
    [
      'not_required',
      { totalPages: 0, indexedPages: 0, reviewPages: 0, failedPages: 0 },
    ],
    [
      'ready',
      { totalPages: 3, indexedPages: 2, reviewPages: 0, failedPages: 0 },
    ],
    [
      'partial',
      { totalPages: 3, indexedPages: 1, reviewPages: 0, failedPages: 1 },
    ],
    [
      'needs_review',
      { totalPages: 3, indexedPages: 1, reviewPages: 2, failedPages: 0 },
    ],
    [
      'failed',
      { totalPages: 3, indexedPages: 0, reviewPages: 0, failedPages: 3 },
    ],
  ])('accepts the safe %s aggregate', (status, counts) => {
    expect(
      parseBookSummary(summary({ indexAggregate: { status, ...counts } })),
    ).toMatchObject({ indexAggregate: { status, ...counts } });
  });

  it.each([
    [
      'missing aggregate',
      (() => {
        const { indexAggregate: _aggregate, ...value } = summary();
        return value;
      })(),
    ],
    [
      'unknown aggregate status',
      summary({
        indexAggregate: {
          status: 'running',
          totalPages: 1,
          indexedPages: 0,
          reviewPages: 0,
          failedPages: 0,
        },
      }),
    ],
    [
      'negative page count',
      summary({
        indexAggregate: {
          status: 'ready',
          totalPages: -1,
          indexedPages: 0,
          reviewPages: 0,
          failedPages: 0,
        },
      }),
    ],
    [
      'non-integer page count',
      summary({
        indexAggregate: {
          status: 'ready',
          totalPages: 1.5,
          indexedPages: 0,
          reviewPages: 0,
          failedPages: 0,
        },
      }),
    ],
    [
      'inconsistent counts',
      summary({
        indexAggregate: {
          status: 'ready',
          totalPages: 1,
          indexedPages: 2,
          reviewPages: 0,
          failedPages: 0,
        },
      }),
    ],
    [
      'extra aggregate data',
      summary({
        indexAggregate: {
          status: 'not_required',
          totalPages: 0,
          indexedPages: 0,
          reviewPages: 0,
          failedPages: 0,
          sourcePath: 'forbidden',
        },
      }),
    ],
    ['extra book data', summary({ sourceHash: 'forbidden' })],
  ])('rejects %s', (_reason, value) => {
    expect(() => parseBookSummary(value)).toThrow();
  });
});
