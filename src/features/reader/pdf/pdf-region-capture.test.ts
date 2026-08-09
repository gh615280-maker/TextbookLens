import { beforeEach, describe, expect, it, vi } from 'vitest';

import { capturePdfRegion } from './pdf-region-capture';

const viewport = {
  width: 600,
  height: 800,
  convertToViewportRectangle: (rect: [number, number, number, number]) => [
    rect[0],
    800 - rect[1],
    rect[2],
    800 - rect[3],
  ],
};

describe('capturePdfRegion', () => {
  beforeEach(() => vi.restoreAllMocks());

  it('reconstructs deterministic reading order and hashes normalized reliable text without pixels', async () => {
    const result = await capturePdfRegion(
      {
        page: 2,
        rect: { x: 0, y: 0, width: 1, height: 1 },
        viewport,
        textItems: [
          item('second line has enough useful textbook characters', 40, 650),
          item('first line has enough useful textbook characters', 40, 750),
        ],
        canvas: null,
      },
      vi.fn(() => {
        throw new Error('confirmation must not run');
      }),
    );
    expect(result).toMatchObject({
      text: 'first line has enough useful textbook characters second line has enough useful textbook characters',
      capture: null,
      anchor: {
        kind: 'region',
        region: {
          locator: { format: 'pdf', page: 2 },
          contentSha256: expect.stringMatching(/^[0-9a-f]{64}$/),
        },
      },
    });
  });

  it('supports the PDF.js 6 point-only viewport contract', async () => {
    const result = await capturePdfRegion(
      {
        page: 1,
        rect: { x: 0, y: 0, width: 1, height: 1 },
        viewport: {
          width: 600,
          height: 800,
          convertToViewportPoint: (x, y) => [x, 800 - y],
        },
        textItems: [
          item(
            'point viewport has enough reliable textbook characters',
            40,
            750,
          ),
        ],
        canvas: null,
      },
      vi.fn(() => {
        throw new Error('confirmation must not run');
      }),
    );
    expect(result).toMatchObject({
      text: 'point viewport has enough reliable textbook characters',
      capture: null,
    });
  });

  it('does not crop mixed/visual content when confirmation is refused', async () => {
    const canvas = document.createElement('canvas');
    const drawImage = vi.fn();
    vi.spyOn(document, 'createElement').mockImplementation(((tag: string) =>
      tag === 'canvas'
        ? ({
            width: 0,
            height: 0,
            getContext: () => ({ drawImage }),
          } as unknown as HTMLCanvasElement)
        : document.createElementNS(
            'http://www.w3.org/1999/xhtml',
            tag,
          )) as typeof document.createElement);
    await expect(
      capturePdfRegion(
        {
          page: 1,
          rect: { x: 0, y: 0, width: 1, height: 1 },
          viewport,
          textItems: [],
          canvas,
        },
        () => false,
      ),
    ).resolves.toBeNull();
    expect(drawImage).not.toHaveBeenCalled();
  });

  it('crops bounded pixels only after confirmation and wipes them on release', async () => {
    const source = Object.assign(document.createElement('canvas'), {
      width: 5000,
      height: 4000,
    });
    const drawImage = vi.fn();
    const target = {
      width: 0,
      height: 0,
      getContext: () => ({ drawImage }),
      toBlob(callback: (blob: Blob) => void) {
        callback(new Blob([new Uint8Array([1, 2, 3])], { type: 'image/png' }));
      },
    };
    vi.spyOn(document, 'createElement').mockImplementation(((tag: string) =>
      tag === 'canvas'
        ? (target as unknown as HTMLCanvasElement)
        : document.createElementNS(
            'http://www.w3.org/1999/xhtml',
            tag,
          )) as typeof document.createElement);
    const confirm = vi.fn(() => true);
    const result = await capturePdfRegion(
      {
        page: 1,
        rect: { x: 0.1, y: 0.1, width: 0.8, height: 0.8 },
        viewport,
        textItems: [],
        canvas: source,
      },
      confirm,
    );
    expect(confirm).toHaveBeenCalledOnce();
    expect(result?.capture).toMatchObject({
      mimeType: 'image/png',
      width: expect.any(Number),
      height: expect.any(Number),
    });
    expect(
      (result?.capture?.width ?? 0) * (result?.capture?.height ?? 0),
    ).toBeLessThanOrEqual(2_000_000);
    const bytes = result!.capture!.bytes;
    result!.capture!.release();
    expect([...bytes]).toEqual([0, 0, 0]);
  });

  it('maps the selected page box through the rendered canvas at CSS zoom', async () => {
    const source = Object.assign(document.createElement('canvas'), {
      width: 1200,
      height: 1600,
    });
    source.getBoundingClientRect = () =>
      ({
        left: 118,
        top: 68,
        right: 718,
        bottom: 868,
        width: 600,
        height: 800,
      }) as DOMRect;
    const pageElement = document.createElement('div');
    pageElement.getBoundingClientRect = () =>
      ({
        left: 100,
        top: 50,
        right: 736,
        bottom: 886,
        width: 636,
        height: 836,
      }) as DOMRect;
    const drawImage = vi.fn();
    const target = {
      width: 0,
      height: 0,
      getContext: () => ({ drawImage }),
      toBlob(callback: (blob: Blob) => void) {
        callback(
          new Blob(
            [
              new Uint8Array([
                0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 1, 2, 3,
              ]),
            ],
            { type: 'image/png' },
          ),
        );
      },
    };
    vi.spyOn(document, 'createElement').mockImplementation(((tag: string) =>
      tag === 'canvas'
        ? (target as unknown as HTMLCanvasElement)
        : document.createElementNS(
            'http://www.w3.org/1999/xhtml',
            tag,
          )) as typeof document.createElement);

    await capturePdfRegion(
      {
        page: 1,
        rect: {
          x: 118 / 636,
          y: 118 / 836,
          width: 300 / 636,
          height: 400 / 836,
        },
        pageElement,
        viewport,
        textItems: [],
        canvas: source,
      },
      () => true,
    );

    expect(drawImage).toHaveBeenCalledWith(
      source,
      200,
      200,
      600,
      800,
      0,
      0,
      600,
      800,
    );
  });

  it('materializes no bytes when an aborted confirmation resolves late', async () => {
    const controller = new AbortController();
    const late = deferred<boolean>();
    const promise = capturePdfRegion(
      {
        page: 1,
        rect: { x: 0, y: 0, width: 1, height: 1 },
        viewport,
        textItems: [],
        canvas: document.createElement('canvas'),
      },
      () => late.promise,
      controller.signal,
    );
    controller.abort();
    late.resolve(true);
    await expect(promise).rejects.toMatchObject({ name: 'AbortError' });
  });
});

function item(str: string, x: number, y: number) {
  return { str, transform: [20, 0, 0, 20, x, y] as const, width: 500 };
}
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => (resolve = done));
  return { promise, resolve };
}
