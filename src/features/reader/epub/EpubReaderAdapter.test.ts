import { describe, expect, it, vi } from 'vitest';
import type { ReaderAdapterEvents } from '../contracts';
import { EpubReaderAdapter } from './EpubReaderAdapter';
import { hashEpubRegionElement } from './epub-region-capture';

describe('EpubReaderAdapter', () => {
  it('uses persisted EPUB IDs for selection and progress after empty spine entries', async () => {
    const id = '33333333-3333-4333-8333-333333333333';
    const cfi = 'epubcfi(/6/38!/4/2/1:0)';
    const document = new DOMParser().parseFromString(
      '<body><p>synthetic selection</p></body>',
      'text/html',
    );
    const range = document.createRange();
    range.selectNodeContents(document.querySelector('p')!);
    const handlers = new Map<string, (...args: unknown[]) => void>();
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
      renderTo: () => rendition,
      getRange: async () => range,
      destroy: vi.fn(),
      spine: { get: () => ({ index: 18 }) },
    };
    const onSelection = vi.fn();
    const onProgress = vi.fn();
    const adapter = new EpubReaderAdapter(
      window.document.body,
      {
        onSelection,
        onProgress,
        onMarkerActivate: vi.fn(),
        onFailure: vi.fn(),
      },
      () => book,
    );
    const source = {
      kind: 'document_bytes' as const,
      bytes: new ArrayBuffer(1),
      sectionBindings: [
        {
          id,
          locator: {
            format: 'epub' as const,
            cfi: 'epubcfi(/6/38!/4/2)',
            sectionId: id,
          },
        },
      ],
    };
    await adapter.open(source);
    handlers.get('relocated')?.({ start: { cfi } });
    expect(adapter.getProgress().locator).toEqual({
      format: 'epub',
      cfi,
      sectionId: id,
    });
    handlers.get('selected')?.(cfi);
    await vi.waitFor(() =>
      expect(onSelection).toHaveBeenCalledWith(
        expect.objectContaining({
          anchor: expect.objectContaining({
            sectionId: id,
            locator: { format: 'epub', cfi, sectionId: id },
          }),
        }),
      ),
    );
    handlers.get('relocated')?.({ start: { cfi: 'epubcfi(/6/2!/4/2)' } });
    expect(adapter.getProgress().locator).toBeNull();
    adapter.dispose();
  });
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
    const primaryText = sectionDocument.querySelector('p')!.firstChild!;
    primaryRange.setStart(primaryText, 7);
    primaryRange.setEnd(primaryText, 13);
    const getRange = vi.fn(async (cfi: string) => {
      void cfi;
      return primaryRange;
    });
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
        kind: 'text' as const,
        selection: {
          locator: {
            format: 'epub' as const,
            cfi: 'epubcfi(/6/4)',
            sectionId: 'section',
          },
          quote: { exact: 'target', prefix: 'prefix ', suffix: ' suffix' },
          sectionId: 'section',
        },
      },
    };
    expect(await adapter.showAnnotations([marker])).toEqual([
      { annotationId: 'epub', relocationStatus: 'primary' },
    ]);
    const regionMarker = {
      id: 'epub-region',
      kind: 'note' as const,
      conversationId: null,
      label: 'View personal note marker',
      relocationStatus: 'primary' as const,
      anchor: {
        kind: 'region' as const,
        region: {
          locator: {
            format: 'epub' as const,
            sectionId: 'spine-0',
            cfi: 'epubcfi(/6/4)',
          },
          rect: { x: 0.1, y: 0.1, width: 0.3, height: 0.2 },
          contentSha256: await hashEpubRegionElement(
            sectionDocument.querySelector('p')!,
          ),
          textFallback: {
            exact: 'target',
            prefix: 'prefix ',
            suffix: ' suffix',
          },
        },
      },
    };
    expect(await adapter.showAnnotations([regionMarker])).toEqual([
      { annotationId: 'epub-region', relocationStatus: 'primary' },
    ]);
    getRange.mockImplementation(async (cfi: string) => {
      if (cfi === 'epubcfi(/6/4)') throw new Error('stale CFI');
      return primaryRange;
    });
    expect(await adapter.showAnnotations([marker])).toEqual([
      { annotationId: 'epub', relocationStatus: 'fallback' },
    ]);
    expect(await adapter.showAnnotations([regionMarker])).toEqual([
      { annotationId: 'epub-region', relocationStatus: 'fallback' },
    ]);
    expect(annotations.remove).toHaveBeenCalledWith(
      'epubcfi(/6/4)',
      'highlight',
    );
    sectionDocument.body.innerHTML = '<p>target target</p>';
    expect(
      await adapter.showAnnotations([
        {
          ...regionMarker,
          anchor: {
            ...regionMarker.anchor,
            region: {
              ...regionMarker.anchor.region,
              textFallback: { exact: 'target', prefix: '', suffix: '' },
            },
          },
        },
      ]),
    ).toEqual([
      { annotationId: 'epub-region', relocationStatus: 'unresolved' },
    ]);
    expect(
      await adapter.showAnnotations([
        {
          ...marker,
          anchor: {
            ...marker.anchor,
            selection: {
              ...marker.anchor.selection,
              quote: { exact: 'target', prefix: '', suffix: '' },
            },
          },
        },
      ]),
    ).toEqual([{ annotationId: 'epub', relocationStatus: 'unresolved' }]);
    expect(document.body.querySelector('.epub-reader-markers')).toBeNull();
    expect(failure).toHaveBeenCalledWith(
      expect.objectContaining({ code: 'ANCHOR_NOT_FOUND' }),
    );
  });

  it('captures a keyboard-accessible iframe region without pixels for reliable text', async () => {
    const setup = await openRegionAdapter(
      '<p>Reliable EPUB region text with enough useful characters.</p>',
    );
    const confirm = vi.fn(() => {
      throw new Error('confirmation must not run');
    });
    const pending = setup.adapter.beginRegionSelection({
      confirmVisualCapture: confirm,
    });
    setup.element.dispatchEvent(pointer(setup.window, 'pointerdown', 10, 10));
    setup.element.dispatchEvent(pointer(setup.window, 'pointerup', 70, 70));
    await expect(pending).resolves.toMatchObject({
      sectionId: 'spine-0',
      cfi: 'epubcfi(/6/2!/4/2)',
      rect: { x: 0.1, y: 0.1, width: 0.6, height: 0.6 },
      capture: null,
      anchor: {
        kind: 'region',
        region: {
          locator: {
            format: 'epub',
            sectionId: 'spine-0',
            cfi: 'epubcfi(/6/2!/4/2)',
          },
          contentSha256: expect.stringMatching(/^[0-9a-f]{64}$/),
        },
      },
    });
    expect(confirm).not.toHaveBeenCalled();
    expect(setup.iframe.getAttribute('sandbox')).toBe('allow-same-origin');
    expect(setup.iframe.getAttribute('sandbox')).not.toContain('allow-scripts');
    setup.adapter.dispose();
    setup.root.remove();
  });

  it('cancels region mode on Escape and pointer cancellation with stable codes', async () => {
    const setup = await openRegionAdapter(
      '<p>Reliable EPUB region text with enough useful characters.</p>',
    );
    const escape = setup.adapter.beginRegionSelection({
      confirmVisualCapture: () => false,
    });
    setup.document.dispatchEvent(
      new setup.window.KeyboardEvent('keydown', { key: 'Escape' }),
    );
    await expect(escape).rejects.toMatchObject({
      instructionCode: 'epub_region_cancelled',
    });
    const cancelled = setup.adapter.beginRegionSelection({
      confirmVisualCapture: () => false,
    });
    setup.document.dispatchEvent(new setup.window.Event('pointercancel'));
    await expect(cancelled).rejects.toMatchObject({
      instructionCode: 'epub_region_cancelled',
    });
    setup.adapter.dispose();
    setup.root.remove();
  });

  it('rejects a tiny element rect and a visual confirmation that returns after dispose', async () => {
    const text = await openRegionAdapter(
      '<p>Reliable EPUB region text with enough useful characters.</p>',
    );
    const tiny = text.adapter.beginRegionSelection({
      confirmVisualCapture: () => false,
    });
    text.element.dispatchEvent(pointer(text.window, 'pointerdown', 10, 10));
    text.element.dispatchEvent(pointer(text.window, 'pointerup', 17, 70));
    await expect(tiny).rejects.toMatchObject({
      instructionCode: 'epub_region_too_small',
    });
    text.adapter.dispose();
    text.root.remove();

    const visual = await openRegionAdapter(
      '<figure><img src="data:image/png;base64,iVBORw0KGgo=" alt="diagram"></figure>',
    );
    const image = visual.element.querySelector('img')!;
    setBounds(image, 0, 0, 100, 100);
    Object.defineProperties(image, {
      naturalWidth: { value: 100 },
      naturalHeight: { value: 100 },
    });
    const confirmation = deferred<boolean>();
    const confirm = vi.fn(() => confirmation.promise);
    const create = vi.spyOn(visual.document, 'createElement');
    const pending = visual.adapter.beginRegionSelection({
      confirmVisualCapture: confirm,
    });
    image.dispatchEvent(pointer(visual.window, 'pointerdown', 10, 10));
    image.dispatchEvent(pointer(visual.window, 'pointerup', 70, 70));
    await vi.waitFor(() => expect(confirm).toHaveBeenCalledOnce());
    visual.adapter.dispose();
    confirmation.resolve(true);
    await expect(pending).rejects.toMatchObject({
      instructionCode: 'epub_region_cancelled',
    });
    await Promise.resolve();
    expect(create.mock.calls.filter(([tag]) => tag === 'canvas')).toHaveLength(
      0,
    );
    visual.root.remove();
  });
});

async function openRegionAdapter(html: string) {
  const root = document.createElement('div');
  document.body.append(root);
  let contentHook!: (contents: {
    document: Document;
    sectionIndex: number;
    cfiFromNode(node: Node): string;
  }) => void;
  const rendition = {
    display: vi.fn(async () => {}),
    on: vi.fn(),
    off: vi.fn(),
    destroy: vi.fn(),
    annotations: { add: vi.fn(), remove: vi.fn() },
    hooks: {
      content: {
        register: vi.fn(
          (handler: typeof contentHook) => (contentHook = handler),
        ),
      },
    },
  };
  const book = {
    open: vi.fn(async () => {}),
    ready: Promise.resolve(),
    renderTo: vi.fn(() => rendition),
    getRange: vi.fn(),
    destroy: vi.fn(),
    spine: { get: vi.fn(() => ({ index: 0 })) },
  };
  const adapter = new EpubReaderAdapter(
    root,
    {
      onSelection: vi.fn(),
      onProgress: vi.fn(),
      onMarkerActivate: vi.fn(),
      onFailure: vi.fn(),
    },
    () => book,
  );
  await adapter.open({ kind: 'document_bytes', bytes: new ArrayBuffer(1) });
  const iframe = document.createElement('iframe');
  root.append(iframe);
  const frameDocument = iframe.contentDocument!;
  frameDocument.body.innerHTML = html;
  const element = frameDocument.body.firstElementChild as HTMLElement;
  setBounds(element, 0, 0, 100, 100);
  contentHook({
    document: frameDocument,
    sectionIndex: 0,
    cfiFromNode: () => 'epubcfi(/6/2!/4/2)',
  });
  return {
    adapter,
    root,
    iframe,
    document: frameDocument,
    window: iframe.contentWindow! as Window & typeof globalThis,
    element,
  };
}

function pointer(
  window: Window & typeof globalThis,
  type: string,
  clientX: number,
  clientY: number,
) {
  return new window.MouseEvent(type, {
    bubbles: true,
    button: 0,
    clientX,
    clientY,
  });
}

function setBounds(
  element: Element,
  left: number,
  top: number,
  width: number,
  height: number,
) {
  element.getBoundingClientRect = () =>
    ({
      left,
      top,
      right: left + width,
      bottom: top + height,
      width,
      height,
      x: left,
      y: top,
      toJSON: () => ({}),
    }) as DOMRect;
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}
