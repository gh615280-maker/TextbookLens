import { describe, expect, it, vi } from 'vitest';
import type { ReaderAdapterEvents } from '../contracts';
import { EpubReaderAdapter } from './EpubReaderAdapter';

describe('EpubReaderAdapter', () => {
  it('opens byte data in continuous vertical flow, restores CFI, and destroys rendition/book', async () => {
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
    expect(book.renderTo).toHaveBeenCalledWith(document.body, {
      width: '100%',
      height: '100%',
      manager: 'continuous',
      flow: 'scrolled',
    });
    expect(display).toHaveBeenCalledWith('epubcfi(/6/4)');
    adapter.dispose();
    expect(destroyRendition).toHaveBeenCalledOnce();
    expect(destroyBook).toHaveBeenCalledOnce();
  });

  it('retains CFI location, selection, search, and progress behavior in continuous flow', async () => {
    const sectionDocument = new DOMParser().parseFromString(
      '<body><p>continuous selection</p></body>',
      'text/html',
    );
    const range = sectionDocument.createRange();
    range.selectNodeContents(sectionDocument.querySelector('p')!);
    const handlers = new Map<string, (...args: unknown[]) => void>();
    const locations = {
      generate: vi.fn(async () => {}),
      percentageFromCfi: vi.fn(() => 0.375),
    };
    const rendition = {
      display: vi.fn(async () => {}),
      on: vi.fn((name: string, handler: (...args: unknown[]) => void) => {
        handlers.set(name, handler);
      }),
      off: vi.fn(),
      destroy: vi.fn(),
      annotations: { add: vi.fn() },
      hooks: { content: { register: vi.fn() } },
    };
    const book = {
      open: vi.fn(async () => {}),
      ready: Promise.resolve(),
      renderTo: vi.fn(() => rendition),
      getRange: vi.fn(async () => range),
      destroy: vi.fn(),
      spine: { get: vi.fn(() => ({ index: 4 })) },
      locations,
    };
    const onSelection = vi.fn();
    const onProgress = vi.fn();
    const adapter = new EpubReaderAdapter(
      document.body,
      {
        onSelection,
        onProgress,
        onMarkerActivate: vi.fn(),
        onFailure: vi.fn(),
      },
      () => book,
    );

    await adapter.open({ kind: 'document_bytes', bytes: new ArrayBuffer(1) });
    await vi.waitFor(() =>
      expect(locations.generate).toHaveBeenCalledWith(1024),
    );

    handlers.get('relocated')?.({
      start: { cfi: 'epubcfi(/6/8!/4/2:0)' },
    });
    expect(onProgress).toHaveBeenCalledWith({
      fraction: 0.375,
      locator: {
        format: 'epub',
        cfi: 'epubcfi(/6/8!/4/2:0)',
        sectionId: 'spine-4',
      },
    });

    handlers.get('selected')?.('epubcfi(/6/8!/4/2:0)');
    await vi.waitFor(() =>
      expect(onSelection).toHaveBeenCalledWith(
        expect.objectContaining({
          text: 'continuous selection',
          anchor: expect.objectContaining({
            locator: expect.objectContaining({
              format: 'epub',
              cfi: 'epubcfi(/6/8!/4/2:0)',
              sectionId: 'spine-4',
            }),
          }),
        }),
      ),
    );
    await expect(adapter.search()).resolves.toEqual([]);
    adapter.dispose();
    expect(rendition.off).toHaveBeenCalledTimes(2);
    expect(rendition.destroy).toHaveBeenCalledOnce();
    expect(book.destroy).toHaveBeenCalledOnce();
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
