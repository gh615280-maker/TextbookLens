import { beforeEach, describe, expect, it, vi } from 'vitest';

const render = vi.fn();
const getPage = vi.fn();
const pageCleanup = vi.fn();
const cleanup = vi.fn();
const destroy = vi.fn();

vi.mock('pdfjs-dist/legacy/build/pdf.mjs', () => ({
  getDocument: vi.fn(() => ({
    promise: Promise.resolve({ getPage, cleanup }),
    destroy,
  })),
}));

import {
  PdfPageRenderError,
  releaseRenderedPdfPage,
  renderPdfPageLocally,
} from './pdf-page-renderer';

describe('renderPdfPageLocally', () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    render.mockReset().mockReturnValue({ promise: Promise.resolve() });
    pageCleanup.mockReset();
    cleanup.mockReset();
    destroy.mockReset();
    getPage.mockReset().mockResolvedValue({
      getViewport: ({ scale }: { scale: number }) => ({
        width: 1000 * scale,
        height: 500 * scale,
      }),
      render,
      cleanup: pageCleanup,
    });
    installCanvasStub();
  });

  it('renders only a bounded PNG with fixed background and rotation', async () => {
    const page = await renderPdfPageLocally(new ArrayBuffer(8), 1, {
      maxDimension: 1000,
      maxDecodedPixels: 1_000_000,
      maxEncodedBytes: 1024,
      maxTotalEncodedBytes: 2048,
    });

    expect(page).toMatchObject({
      schemaVersion: 1,
      mimeType: 'image/png',
      width: 1000,
      height: 500,
      decodedPixelCount: 500_000,
      encodedByteLength: 24,
    });
    expect(render).toHaveBeenCalledWith(
      expect.objectContaining({ background: '#ffffff', intent: 'display' }),
    );
    expect(getPage).toHaveBeenCalledWith(1);
    expect(pageCleanup).toHaveBeenCalledOnce();
    expect(cleanup).toHaveBeenCalledOnce();
    expect(destroy).toHaveBeenCalledOnce();

    releaseRenderedPdfPage(page);
    expect([...page.bytes]).toEqual(Array(page.bytes.length).fill(0));
  });

  it('deterministically downscales before creating an oversized raster', async () => {
    await renderPdfPageLocally(new ArrayBuffer(8), 1, { maxDimension: 600 });

    const options = render.mock.calls[0]?.[0] as {
      viewport: { width: number; height: number };
    };
    expect(options.viewport.width).toBeCloseTo(600);
    expect(options.viewport.height).toBeCloseTo(300);
  });

  it('fails before rendering when the total byte budget is exhausted', async () => {
    await expect(
      renderPdfPageLocally(new ArrayBuffer(8), 1, {
        maxTotalEncodedBytes: 10,
        alreadyEncodedBytes: 10,
      }),
    ).rejects.toEqual(
      expect.objectContaining({ code: 'RENDER_LIMIT_EXCEEDED' }),
    );
    expect(render).not.toHaveBeenCalled();
  });

  it('rejects invalid page numbers', async () => {
    await expect(
      renderPdfPageLocally(new ArrayBuffer(8), 0),
    ).rejects.toBeInstanceOf(PdfPageRenderError);
  });
});

function installCanvasStub(): void {
  vi.spyOn(document, 'createElement').mockImplementation(((tagName: string) => {
    if (tagName !== 'canvas')
      return document.createElementNS('http://www.w3.org/1999/xhtml', tagName);
    const canvas = {
      width: 0,
      height: 0,
      getContext: () => ({}),
      toBlob: (callback: (blob: Blob) => void) =>
        callback(
          new Blob(
            [
              pngWithDimensions(canvas.width, canvas.height)
                .buffer as ArrayBuffer,
            ],
            { type: 'image/png' },
          ),
        ),
    };
    return canvas as unknown as HTMLCanvasElement;
  }) as typeof document.createElement);
  vi.stubGlobal('crypto', {
    subtle: { digest: vi.fn(async () => new Uint8Array(32).buffer) },
  });
}

function pngWithDimensions(width: number, height: number): Uint8Array {
  const bytes = new Uint8Array(24);
  bytes.set([137, 80, 78, 71, 13, 10, 26, 10]);
  new DataView(bytes.buffer).setUint32(16, width);
  new DataView(bytes.buffer).setUint32(20, height);
  return bytes;
}
