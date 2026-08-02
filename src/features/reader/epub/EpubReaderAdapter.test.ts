import { describe, expect, it, vi } from 'vitest';
import type { ReaderAdapterEvents } from '../contracts';
import { EpubReaderAdapter } from './EpubReaderAdapter';

describe('EpubReaderAdapter', () => {
  it('opens byte data, restores CFI and destroys rendition/book', async () => {
    const destroyBook = vi.fn(); const destroyRendition = vi.fn(); const display = vi.fn(async () => {});
    const rendition = { display, on: vi.fn(), off: vi.fn(), destroy: destroyRendition, annotations: { add: vi.fn() }, hooks: { content: { register: vi.fn() } } };
    const book = { open: vi.fn(async () => {}), ready: Promise.resolve(), renderTo: vi.fn(() => rendition), getRange: vi.fn(), destroy: destroyBook, spine: { get: vi.fn(() => ({ index: 2 })) } };
    const events: ReaderAdapterEvents = { onSelection: vi.fn(), onProgress: vi.fn(), onMarkerActivate: vi.fn(), onFailure: vi.fn() };
    const adapter = new EpubReaderAdapter(document.body, events, () => book);
    await adapter.open({ kind: 'document_bytes', bytes: new ArrayBuffer(1) }, { format: 'epub', cfi: 'epubcfi(/6/4)', sectionId: 'spine-2' });
    expect(display).toHaveBeenCalledWith('epubcfi(/6/4)'); adapter.dispose(); expect(destroyRendition).toHaveBeenCalledOnce(); expect(destroyBook).toHaveBeenCalledOnce();
  });
});
