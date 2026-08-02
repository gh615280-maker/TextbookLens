import { describe, expect, it, vi } from 'vitest';
import type { ReaderAdapterEvents } from '../contracts';
import { PdfReaderAdapter } from './PdfReaderAdapter';

const events: ReaderAdapterEvents = {
  onSelection: vi.fn(),
  onProgress: vi.fn(),
  onMarkerActivate: vi.fn(),
  onFailure: vi.fn(),
};

describe('PdfReaderAdapter', () => {
  it('opens copied bytes, navigates pages and releases PDF resources', async () => {
    const cleanup = vi.fn();
    const destroy = vi.fn();
    const viewer = {
      setDocument: vi.fn(),
      cleanup: vi.fn(),
      currentPageNumber: 1,
      currentScale: 1,
      pagesRotation: 0,
    };
    const adapter = new PdfReaderAdapter(
      document.body,
      events,
      () => ({ promise: Promise.resolve({ numPages: 3, cleanup }), destroy }),
      () => viewer,
    );
    await adapter.open(
      { kind: 'document_bytes', bytes: new Uint8Array([1]).buffer },
      { format: 'pdf', startPage: 2, endPage: 2, rectsByPage: null },
    );
    expect(adapter.getProgress()).toMatchObject({
      fraction: 2 / 3,
      locator: { startPage: 2 },
    });
    expect(
      await adapter.navigate({
        format: 'pdf',
        startPage: 3,
        endPage: 3,
        rectsByPage: null,
      }),
    ).toEqual({ found: true });
    adapter.dispose();
    expect(viewer.setDocument).toHaveBeenCalledOnce();
    expect(viewer.cleanup).toHaveBeenCalledOnce();
    expect(cleanup).toHaveBeenCalledOnce();
    expect(destroy).toHaveBeenCalledOnce();
  });

  it('reports primary, unique page-confined fallback, and ambiguous unresolved recovery', async () => {
    const viewer = {
      setDocument: vi.fn(),
      cleanup: vi.fn(),
      currentPageNumber: 1,
      currentScale: 1,
      pagesRotation: 0,
    };
    const adapter = new PdfReaderAdapter(
      document.body,
      events,
      () => ({
        promise: Promise.resolve({ numPages: 1, cleanup: vi.fn() }),
        destroy: vi.fn(),
      }),
      () => viewer,
    );
    await adapter.open({ kind: 'document_bytes', bytes: new ArrayBuffer(1) });
    const page = document.createElement('div');
    page.dataset.pageNumber = '1';
    page.textContent = 'prefix target suffix';
    Object.defineProperty(page, 'getBoundingClientRect', {
      value: () => ({ left: 0, top: 0, width: 100, height: 100 }),
    });
    document.querySelector('.pdf-viewer')!.append(page);
    const getClientRects = Range.prototype.getClientRects;
    Range.prototype.getClientRects = () =>
      [{ left: 1, top: 1, width: 10, height: 10 }] as unknown as DOMRectList;
    const base = {
      id: 'marker',
      kind: 'note' as const,
      label: '查看个人批注',
      relocationStatus: 'primary' as const,
      anchor: {
        locator: {
          format: 'pdf' as const,
          startPage: 1,
          endPage: 1,
          rectsByPage: null,
        },
        quote: { exact: 'target', prefix: 'prefix ', suffix: ' suffix' },
        sectionId: 'section',
      },
    };
    expect(
      await adapter.showAnnotations([
        {
          ...base,
          anchor: {
            ...base.anchor,
            locator: {
              ...base.anchor.locator,
              rectsByPage: { 1: [{ x: 0.1, y: 0.1, width: 0.2, height: 0.1 }] },
            },
          },
        },
      ]),
    ).toEqual([{ annotationId: 'marker', relocationStatus: 'primary' }]);
    expect(await adapter.showAnnotations([base])).toEqual([
      { annotationId: 'marker', relocationStatus: 'fallback' },
    ]);
    page.textContent = 'target target';
    expect(
      await adapter.showAnnotations([
        {
          ...base,
          anchor: {
            ...base.anchor,
            quote: { exact: 'target', prefix: '', suffix: '' },
          },
        },
      ]),
    ).toEqual([{ annotationId: 'marker', relocationStatus: 'unresolved' }]);
    expect(document.body.querySelector('.pdf-reader-markers')).toBeNull();
    Range.prototype.getClientRects = getClientRects;
  });
});
