import { describe, expect, it, vi } from 'vitest';
import type { ReaderAdapterEvents } from '../contracts';
import { EpubReaderAdapter } from './EpubReaderAdapter';

describe('EpubReaderAdapter', () => {
  it('opens byte data, restores CFI and destroys rendition/book', async () => {
    const destroyBook = vi.fn();
    const destroyRendition = vi.fn();
    const display = vi.fn(async () => {});
    const rendition = {
      display,
      on: vi.fn(),
      off: vi.fn(),
      destroy: destroyRendition,
      annotations: { add: vi.fn() },
      hooks: { content: { register: vi.fn() } },
    };
    const book = {
      open: vi.fn(async () => {}),
      ready: Promise.resolve(),
      renderTo: vi.fn(() => rendition),
      getRange: vi.fn(),
      destroy: destroyBook,
      spine: { get: vi.fn(() => ({ index: 2 })) },
    };
    const events: ReaderAdapterEvents = {
      onSelection: vi.fn(),
      onProgress: vi.fn(),
      onMarkerActivate: vi.fn(),
      onFailure: vi.fn(),
    };
    const adapter = new EpubReaderAdapter(document.body, events, () => book);
    await adapter.open(
      { kind: 'document_bytes', bytes: new ArrayBuffer(1) },
      { format: 'epub', cfi: 'epubcfi(/6/4)', sectionId: 'spine-2' },
    );
    expect(display).toHaveBeenCalledWith('epubcfi(/6/4)');
    adapter.dispose();
    expect(destroyRendition).toHaveBeenCalledOnce();
    expect(destroyBook).toHaveBeenCalledOnce();
  });

  it('uses CFI primary recovery, section-confined unique fallback, and leaves ambiguity unattached', async () => {
    const sectionDocument = new DOMParser().parseFromString(
      '<body><p>prefix target suffix</p></body>',
      'text/html',
    );
    const section = {
      index: 0,
      document: sectionDocument,
      load: vi.fn(async () => {}),
      unload: vi.fn(),
      cfiFromElement: vi.fn(() => 'epubcfi(/6/4!/4/2:1)'),
    };
    const annotations = { add: vi.fn(), remove: vi.fn() };
    const rendition = {
      display: vi.fn(async () => {}),
      on: vi.fn(),
      off: vi.fn(),
      destroy: vi.fn(),
      annotations,
      hooks: { content: { register: vi.fn() } },
    };
    const primaryRange = sectionDocument.createRange();
    primaryRange.selectNodeContents(sectionDocument.querySelector('p')!);
    const getRange = vi.fn(async () => primaryRange);
    const book = {
      open: vi.fn(async () => {}),
      ready: Promise.resolve(),
      renderTo: vi.fn(() => rendition),
      getRange,
      destroy: vi.fn(),
      spine: { get: vi.fn(() => section) },
    };
    const failure = vi.fn();
    const adapter = new EpubReaderAdapter(
      document.body,
      {
        onSelection: vi.fn(),
        onProgress: vi.fn(),
        onMarkerActivate: vi.fn(),
        onFailure: failure,
      },
      () => book,
    );
    await adapter.open({ kind: 'document_bytes', bytes: new ArrayBuffer(1) });
    const marker = {
      id: 'epub',
      kind: 'ai_conversation' as const,
      label: '查看 AI 对话标记',
      relocationStatus: 'primary' as const,
      anchor: {
        locator: {
          format: 'epub' as const,
          cfi: 'epubcfi(/6/4)',
          sectionId: 'section',
        },
        quote: { exact: 'target', prefix: 'prefix ', suffix: ' suffix' },
        sectionId: 'section',
      },
    };
    expect(await adapter.showAnnotations([marker])).toEqual([
      { annotationId: 'epub', relocationStatus: 'primary' },
    ]);
    getRange.mockRejectedValue(new Error('stale CFI'));
    expect(await adapter.showAnnotations([marker])).toEqual([
      { annotationId: 'epub', relocationStatus: 'fallback' },
    ]);
    expect(annotations.remove).toHaveBeenCalledWith(
      'epubcfi(/6/4)',
      'highlight',
    );
    sectionDocument.body.innerHTML = '<p>target target</p>';
    expect(
      await adapter.showAnnotations([
        {
          ...marker,
          anchor: {
            ...marker.anchor,
            quote: { exact: 'target', prefix: '', suffix: '' },
          },
        },
      ]),
    ).toEqual([{ annotationId: 'epub', relocationStatus: 'unresolved' }]);
    expect(document.body.querySelector('.epub-reader-markers')).toBeNull();
    expect(failure).toHaveBeenCalledWith(
      expect.objectContaining({ code: 'ANCHOR_NOT_FOUND' }),
    );
  });
});
