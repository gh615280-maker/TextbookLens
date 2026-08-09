import { describe, expect, it, vi } from 'vitest';
import type { ReaderAdapterEvents } from '../contracts';
import { PdfReaderAdapter } from './PdfReaderAdapter';

const events: ReaderAdapterEvents = {
  onSelection: vi.fn(),
  onProgress: vi.fn(),
  onMarkerActivate: vi.fn(),
  onMarkersResolved: vi.fn(),
  onFailure: vi.fn(),
};

describe('PdfReaderAdapter', () => {
  it('opens copied bytes, navigates pages and releases PDF resources', async () => {
    const cleanup = vi.fn();
    const destroy = vi.fn();
    const viewer = {
      setDocument: vi.fn(),
      cleanup: vi.fn(),
      firstPagePromise: Promise.resolve(),
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
    expect(adapter.getPageCount()).toBe(3);
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

  it('uses native PDF scaling and follows reader setting changes', async () => {
    const assignedScales = [1];
    const viewer = {
      setDocument: vi.fn(),
      cleanup: vi.fn(),
      firstPagePromise: Promise.resolve(),
      currentPageNumber: 1,
      get currentScale() {
        return assignedScales.at(-1) ?? 1;
      },
      set currentScale(scale: number) {
        assignedScales.push(scale);
      },
      pagesRotation: 0,
    };
    const layout = document.createElement('div');
    layout.className = 'reader-layout';
    layout.style.setProperty('--reader-content-zoom', '1.4');
    const container = document.createElement('div');
    layout.append(container);
    document.body.append(layout);
    const adapter = new PdfReaderAdapter(
      container,
      events,
      () => ({
        promise: Promise.resolve({ numPages: 1, cleanup: vi.fn() }),
        destroy: vi.fn(),
      }),
      () => viewer,
    );

    await adapter.open({ kind: 'document_bytes', bytes: new ArrayBuffer(1) });
    expect(assignedScales).toEqual([1, 1.4]);

    layout.style.setProperty('--reader-content-zoom', '2.6');
    await vi.waitFor(() => expect(assignedScales.at(-1)).toBe(2.6));

    adapter.dispose();
    layout.remove();
  });

  it('restores the initial page only after the real viewer readiness boundary', async () => {
    const ready = deferred<void>();
    const assignedPages: number[] = [];
    const viewer = {
      setDocument: vi.fn(),
      cleanup: vi.fn(),
      firstPagePromise: ready.promise,
      get currentPageNumber() {
        return assignedPages.at(-1) ?? 1;
      },
      set currentPageNumber(page: number) {
        assignedPages.push(page);
      },
      currentScale: 1,
      pagesRotation: 0,
    };
    const adapter = new PdfReaderAdapter(
      document.body,
      events,
      () => ({
        promise: Promise.resolve({ numPages: 3, cleanup: vi.fn() }),
        destroy: vi.fn(),
      }),
      () => viewer,
    );

    const opening = adapter.open(
      { kind: 'document_bytes', bytes: new Uint8Array([1]).buffer },
      { format: 'pdf', startPage: 2, endPage: 2, rectsByPage: null },
    );
    await vi.waitFor(() => expect(viewer.setDocument).toHaveBeenCalledOnce());
    expect(assignedPages).toEqual([]);

    ready.resolve();
    await opening;

    expect(assignedPages).toEqual([2]);
    expect(adapter.getProgress()).toMatchObject({
      fraction: 2 / 3,
      locator: { startPage: 2 },
    });
    adapter.dispose();
  });

  it('ignores viewer readiness that arrives after disposal and releases resources once', async () => {
    const ready = deferred<void>();
    const cleanup = vi.fn();
    const destroy = vi.fn();
    const assignedPages: number[] = [];
    const viewer = {
      setDocument: vi.fn(),
      cleanup: vi.fn(),
      firstPagePromise: ready.promise,
      get currentPageNumber() {
        return assignedPages.at(-1) ?? 1;
      },
      set currentPageNumber(page: number) {
        assignedPages.push(page);
      },
      currentScale: 1,
      pagesRotation: 0,
    };
    const adapter = new PdfReaderAdapter(
      document.body,
      events,
      () => ({
        promise: Promise.resolve({ numPages: 3, cleanup }),
        destroy,
      }),
      () => viewer,
    );

    const opening = adapter.open(
      { kind: 'document_bytes', bytes: new Uint8Array([1]).buffer },
      { format: 'pdf', startPage: 2, endPage: 2, rectsByPage: null },
    );
    await vi.waitFor(() => expect(viewer.setDocument).toHaveBeenCalledOnce());
    adapter.dispose();
    ready.resolve();
    await opening;

    expect(assignedPages).toEqual([]);
    expect(viewer.cleanup).toHaveBeenCalledOnce();
    expect(cleanup).toHaveBeenCalledOnce();
    expect(destroy).toHaveBeenCalledOnce();
    expect(document.body).not.toHaveClass('pdf-reader');
    expect(document.body).toBeEmptyDOMElement();
    expect(
      await adapter.navigate({
        format: 'pdf',
        startPage: 2,
        endPage: 2,
        rectsByPage: null,
      }),
    ).toEqual({ found: false });
  });

  it('reports primary visual geometry, unique text fallback, and ambiguous unresolved recovery', async () => {
    const viewer = {
      setDocument: vi.fn(),
      cleanup: vi.fn(),
      firstPagePromise: Promise.resolve(),
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
      value: () => new DOMRect(10, 20, 100, 100),
    });
    document.querySelector('.pdf-viewer')!.append(page);
    const getClientRects = Range.prototype.getClientRects;
    Range.prototype.getClientRects = () =>
      [{ left: 11, top: 21, width: 10, height: 10 }] as unknown as DOMRectList;
    const base = {
      id: 'marker',
      kind: 'note' as const,
      label: '查看个人批注',
      relocationStatus: 'primary' as const,
      anchor: {
        kind: 'text' as const,
        selection: {
          locator: {
            format: 'pdf' as const,
            startPage: 1,
            endPage: 1,
            rectsByPage: null,
          },
          quote: { exact: 'target', prefix: 'prefix ', suffix: ' suffix' },
          sectionId: 'section',
        },
      },
    };
    expect(
      await adapter.showAnnotations([
        {
          ...base,
          anchor: {
            ...base.anchor,
            selection: {
              ...base.anchor.selection,
              locator: {
                ...base.anchor.selection.locator,
                rectsByPage: {
                  1: [{ x: 0.1, y: 0.1, width: 0.2, height: 0.1 }],
                },
              },
            },
          },
        },
      ]),
    ).toEqual([{ annotationId: 'marker', relocationStatus: 'primary' }]);
    expect(
      await adapter.showAnnotations([
        {
          id: 'region-marker',
          kind: 'ai_conversation',
          conversationId: 'conversation',
          label: 'View AI conversation marker',
          relocationStatus: 'primary',
          anchor: {
            kind: 'region',
            region: {
              locator: { format: 'pdf', page: 1 },
              rect: { x: 0.1, y: 0.1, width: 0.2, height: 0.1 },
              contentSha256: await sha256Text('target'),
              textFallback: {
                exact: 'target',
                prefix: 'prefix ',
                suffix: ' suffix',
              },
            },
          },
        },
      ]),
    ).toEqual([{ annotationId: 'region-marker', relocationStatus: 'primary' }]);
    expect(document.body.querySelector('.pdf-marker-rect')).toHaveStyle({
      left: '10px',
      top: '10px',
      width: '20px',
      height: '10px',
    });
    expect(document.body.querySelector('.reader-marker-button')).not.toBeNull();
    expect(
      document.body
        .querySelector('.reader-marker-button')
        ?.closest('[data-page-number]'),
    ).toBe(page);
    expect(document.body.querySelector('.reader-marker-button')).toHaveStyle({
      left: '30px',
      top: '10px',
    });
    expect(events.onMarkersResolved).toHaveBeenCalled();
    expect(
      await adapter.showAnnotations([
        {
          id: 'visual-region-with-fallback',
          kind: 'ai_conversation',
          conversationId: 'conversation',
          label: 'View AI conversation marker',
          relocationStatus: 'primary',
          anchor: {
            kind: 'region',
            region: {
              locator: { format: 'pdf', page: 1 },
              rect: { x: 0.1, y: 0.1, width: 0.2, height: 0.1 },
              contentSha256: 'f'.repeat(64),
              textFallback: {
                exact: 'blue title absent from the rendered text layer',
                prefix: '',
                suffix: '',
              },
            },
          },
        },
      ]),
    ).toEqual([
      {
        annotationId: 'visual-region-with-fallback',
        relocationStatus: 'primary',
      },
    ]);
    const movedRegion = {
      id: 'moved-region-marker',
      kind: 'ai_conversation' as const,
      conversationId: 'conversation',
      label: 'View AI conversation marker',
      relocationStatus: 'primary' as const,
      anchor: {
        kind: 'region' as const,
        region: {
          locator: { format: 'pdf' as const, page: 1 },
          rect: { x: 0.1, y: 0.1, width: 0.2, height: 0.1 },
          contentSha256: 'f'.repeat(64),
          textFallback: {
            exact: 'target',
            prefix: 'prefix ',
            suffix: ' suffix',
          },
        },
      },
    };
    expect(await adapter.showAnnotations([movedRegion])).toEqual([
      { annotationId: 'moved-region-marker', relocationStatus: 'primary' },
    ]);
    expect(await adapter.showAnnotations([base])).toEqual([
      { annotationId: 'marker', relocationStatus: 'fallback' },
    ]);
    page.textContent = 'target target';
    expect(
      await adapter.showAnnotations([
        {
          ...movedRegion,
          anchor: {
            ...movedRegion.anchor,
            region: {
              ...movedRegion.anchor.region,
              textFallback: { exact: 'target', prefix: '', suffix: '' },
            },
          },
        },
      ]),
    ).toEqual([
      { annotationId: 'moved-region-marker', relocationStatus: 'primary' },
    ]);
    expect(
      await adapter.showAnnotations([
        {
          ...base,
          anchor: {
            ...base.anchor,
            selection: {
              ...base.anchor.selection,
              quote: { exact: 'target', prefix: '', suffix: '' },
            },
          },
        },
      ]),
    ).toEqual([{ annotationId: 'marker', relocationStatus: 'unresolved' }]);
    expect(document.body.querySelector('.pdf-reader-markers')).toBeNull();
    Range.prototype.getClientRects = getClientRects;
  });

  it('cancels region mode on Escape and pointer cancellation with a stable code', async () => {
    const adapter = await openRegionAdapter();
    const escape = adapter.beginRegionSelection({
      confirmVisualCapture: () => false,
    });
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }));
    await expect(escape).rejects.toMatchObject({
      instructionCode: 'pdf_region_cancelled',
    });

    const pointerCancel = adapter.beginRegionSelection({
      confirmVisualCapture: () => false,
    });
    window.dispatchEvent(new Event('pointercancel'));
    await expect(pointerCancel).rejects.toMatchObject({
      instructionCode: 'pdf_region_cancelled',
    });
    adapter.dispose();
  });

  it('shows the bounded region while the pointer is dragging', async () => {
    const adapter = await openRegionAdapter();
    appendPage(1, 0);
    const selecting = adapter.beginRegionSelection({
      confirmVisualCapture: () => false,
    });

    document.body.dispatchEvent(pointer('pointerdown', 10, 15));
    window.dispatchEvent(pointer('pointermove', 70, 55));

    expect(
      document.body.querySelector<HTMLElement>('.pdf-region-preview'),
    ).toMatchObject({
      style: expect.objectContaining({
        left: '10px',
        top: '15px',
        width: '60px',
        height: '40px',
      }),
    });

    window.dispatchEvent(new Event('pointercancel'));
    await expect(selecting).rejects.toMatchObject({
      instructionCode: 'pdf_region_cancelled',
    });
    expect(document.body.querySelector('.pdf-region-preview')).toBeNull();
    adapter.dispose();
  });

  it('keeps the preview edges aligned with the pointer at non-default zoom', async () => {
    const adapter = await openRegionAdapter();
    const page = appendPage(1, 45);
    Object.defineProperties(page, {
      offsetWidth: { configurable: true, value: 100 },
      offsetHeight: { configurable: true, value: 100 },
    });
    page.getBoundingClientRect = () =>
      ({
        left: 30,
        top: 45,
        right: 180,
        bottom: 195,
        width: 150,
        height: 150,
      }) as DOMRect;
    const selecting = adapter.beginRegionSelection({
      confirmVisualCapture: () => false,
    });

    page.dispatchEvent(pointer('pointerdown', 45, 60));
    window.dispatchEvent(pointer('pointermove', 120, 105));

    expect(
      page.querySelector<HTMLElement>('.pdf-region-preview'),
    ).toMatchObject({
      style: expect.objectContaining({
        left: '10px',
        top: '10px',
        width: '50px',
        height: '30px',
      }),
    });

    window.dispatchEvent(new Event('pointercancel'));
    await expect(selecting).rejects.toMatchObject({
      instructionCode: 'pdf_region_cancelled',
    });
    adapter.dispose();
  });

  it('rejects a cross-page drag without creating an ambiguous anchor', async () => {
    const adapter = await openRegionAdapter();
    appendPage(1, 0);
    appendPage(2, 120);
    const selecting = adapter.beginRegionSelection({
      confirmVisualCapture: () => false,
    });
    document.body.dispatchEvent(pointer('pointerdown', 20, 20));
    window.dispatchEvent(pointer('pointerup', 20, 140));
    await expect(selecting).rejects.toMatchObject({
      instructionCode: 'pdf_region_cross_page',
    });
    adapter.dispose();
  });

  it('rejects a rotate/zoom rerender race and ignores late text after dispose', async () => {
    const text = deferred<{ items: [] }>();
    const viewport = {
      width: 100,
      height: 100,
      rotation: 0,
      scale: 1,
      convertToViewportRectangle: (rect: number[]) => rect,
    };
    const pageView = {
      viewport,
      pdfPage: { getTextContent: () => text.promise },
      canvas: null,
    };
    const adapter = await openRegionAdapter(() => pageView);
    appendPage(1, 0);
    const selecting = adapter.beginRegionSelection({
      confirmVisualCapture: () => false,
    });
    document.body.dispatchEvent(pointer('pointerdown', 10, 10));
    window.dispatchEvent(pointer('pointerup', 50, 50));
    viewport.rotation = 90;
    text.resolve({ items: [] });
    await expect(selecting).rejects.toMatchObject({
      instructionCode: 'pdf_region_unavailable',
    });

    const rerender = deferred<{ items: [] }>();
    pageView.pdfPage.getTextContent = () => rerender.promise;
    viewport.rotation = 0;
    const stalePage = document.querySelector<HTMLElement>(
      '[data-page-number="1"]',
    )!;
    const afterRerender = adapter.beginRegionSelection({
      confirmVisualCapture: () => false,
    });
    document.body.dispatchEvent(pointer('pointerdown', 10, 10));
    window.dispatchEvent(pointer('pointerup', 50, 50));
    stalePage.remove();
    rerender.resolve({ items: [] });
    await expect(afterRerender).rejects.toMatchObject({
      instructionCode: 'pdf_region_unavailable',
    });

    const late = deferred<{ items: [] }>();
    pageView.pdfPage.getTextContent = () => late.promise;
    appendPage(1, 0);
    const afterDispose = adapter.beginRegionSelection({
      confirmVisualCapture: () => false,
    });
    document.body.dispatchEvent(pointer('pointerdown', 10, 10));
    window.dispatchEvent(pointer('pointerup', 50, 50));
    adapter.dispose();
    late.resolve({ items: [] });
    await expect(afterDispose).rejects.toMatchObject({
      instructionCode: 'pdf_region_cancelled',
    });
    expect(document.body).toBeEmptyDOMElement();
  });
});

async function openRegionAdapter(getPageView?: (index: number) => unknown) {
  const viewer = {
    setDocument: vi.fn(),
    cleanup: vi.fn(),
    firstPagePromise: Promise.resolve(),
    currentPageNumber: 1,
    currentScale: 1,
    pagesRotation: 0,
    getPageView,
  };
  const adapter = new PdfReaderAdapter(
    document.body,
    events,
    () => ({
      promise: Promise.resolve({ numPages: 2, cleanup: vi.fn() }),
      destroy: vi.fn(),
    }),
    () => viewer as never,
  );
  await adapter.open({ kind: 'document_bytes', bytes: new ArrayBuffer(1) });
  return adapter;
}

function appendPage(pageNumber: number, top: number) {
  const page = document.createElement('div');
  page.dataset.pageNumber = String(pageNumber);
  page.getBoundingClientRect = () =>
    ({
      left: 0,
      top,
      right: 100,
      bottom: top + 100,
      width: 100,
      height: 100,
    }) as DOMRect;
  document.querySelector('.pdf-viewer')!.append(page);
  return page;
}

function pointer(type: string, clientX: number, clientY: number) {
  return new MouseEvent(type, {
    bubbles: true,
    button: 0,
    clientX,
    clientY,
  }) as unknown as PointerEvent;
}

function deferred<T>() {
  let resolve!: (value: T | PromiseLike<T>) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

async function sha256Text(value: string): Promise<string> {
  const digest = await crypto.subtle.digest(
    'SHA-256',
    new TextEncoder().encode(value),
  );
  return [...new Uint8Array(digest)]
    .map((byte) => byte.toString(16).padStart(2, '0'))
    .join('');
}
