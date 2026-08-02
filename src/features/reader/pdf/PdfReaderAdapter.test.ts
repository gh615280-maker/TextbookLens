import { describe, expect, it, vi } from 'vitest';
import type { ReaderAdapterEvents } from '../contracts';
import { PdfReaderAdapter } from './PdfReaderAdapter';

const events: ReaderAdapterEvents = { onSelection: vi.fn(), onProgress: vi.fn(), onMarkerActivate: vi.fn(), onFailure: vi.fn() };

describe('PdfReaderAdapter', () => {
  it('opens copied bytes, navigates pages and releases PDF resources', async () => {
    const cleanup = vi.fn(); const destroy = vi.fn();
    const viewer = { setDocument: vi.fn(), cleanup: vi.fn(), currentPageNumber: 1, currentScale: 1, pagesRotation: 0 };
    const adapter = new PdfReaderAdapter(document.body, events, () => ({ promise: Promise.resolve({ numPages: 3, cleanup }), destroy }), () => viewer);
    await adapter.open({ kind: 'document_bytes', bytes: new Uint8Array([1]).buffer }, { format: 'pdf', startPage: 2, endPage: 2, rectsByPage: null });
    expect(adapter.getProgress()).toMatchObject({ fraction: 2 / 3, locator: { startPage: 2 } });
    expect(await adapter.navigate({ format: 'pdf', startPage: 3, endPage: 3, rectsByPage: null })).toEqual({ found: true });
    adapter.dispose();
    expect(viewer.setDocument).toHaveBeenCalledOnce(); expect(viewer.cleanup).toHaveBeenCalledOnce(); expect(cleanup).toHaveBeenCalledOnce(); expect(destroy).toHaveBeenCalledOnce();
  });
});
