import { beforeEach, describe, expect, it, vi } from 'vitest';

import {
  captureEpubRegion,
  hashEpubRegionElement,
  resolveEpubRegionAnchor,
} from './epub-region-capture';

const rect = { x: 0.1, y: 0.1, width: 0.8, height: 0.8 };

describe('captureEpubRegion', () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    document.body.replaceChildren();
  });

  it('returns reliable local text with a tagged lowercase hash and no pixels', async () => {
    const element = appendElement(
      '<p>Reliable local textbook text with enough useful characters.</p>',
    );
    const confirm = vi.fn(() => {
      throw new Error('confirmation must not run');
    });
    const result = await captureEpubRegion(
      {
        sectionId: 'spine-3',
        cfi: 'epubcfi(/6/8!/4/2)',
        element,
        rect,
      },
      confirm,
    );
    expect(result).toMatchObject({
      text: 'Reliable local textbook text with enough useful characters.',
      capture: null,
      anchor: {
        kind: 'region',
        region: {
          locator: {
            format: 'epub',
            sectionId: 'spine-3',
            cfi: 'epubcfi(/6/8!/4/2)',
          },
          rect,
          contentSha256: expect.stringMatching(/^[0-9a-f]{64}$/),
        },
      },
    });
    expect(confirm).not.toHaveBeenCalled();
    const serialized = JSON.stringify(result?.anchor);
    expect(serialized).not.toMatch(
      /data:image|outerHTML|ownerDocument|[a-z]:\\/iu,
    );
  });

  it('relocates by exact section/CFI plus hash across reflow and rejects changed content', async () => {
    const element = appendElement(
      '<p style="font-size: 12px">Stable content across rendition reflow.</p>',
    );
    const hash = await hashEpubRegionElement(element);
    const anchor = {
      locator: {
        format: 'epub' as const,
        sectionId: 'spine-1',
        cfi: 'epubcfi(/6/4!/4/2)',
      },
      rect,
      contentSha256: hash,
      textFallback: null,
    };
    element.remove();
    const relocated = appendElement(
      '<p style="font-size: 28px">Stable content across rendition reflow.</p>',
    );
    setBounds(relocated, { left: -200, top: 400, width: 900, height: 240 });
    const range = document.createRange();
    range.selectNodeContents(relocated.firstChild!);
    const resolver = {
      getRange: vi.fn(async () => range),
      sectionIdForCfi: vi.fn(() => 'spine-1'),
    };
    await expect(resolveEpubRegionAnchor(resolver, anchor)).resolves.toEqual({
      element: relocated,
      rect,
    });
    relocated.textContent = 'Changed rendition content.';
    await expect(resolveEpubRegionAnchor(resolver, anchor)).resolves.toBeNull();
    await expect(
      resolveEpubRegionAnchor(
        { ...resolver, sectionIdForCfi: () => 'spine-2' },
        anchor,
      ),
    ).resolves.toBeNull();
  });

  it('materializes zero canvas bytes when visual confirmation is refused', async () => {
    const element = appendVisual();
    const create = vi.spyOn(document, 'createElement');
    await expect(
      captureEpubRegion(
        {
          sectionId: 'spine-0',
          cfi: 'epubcfi(/6/2!/4/2)',
          element,
          rect,
        },
        () => false,
      ),
    ).resolves.toBeNull();
    expect(create.mock.calls.filter(([tag]) => tag === 'canvas')).toHaveLength(
      0,
    );
  });

  it('captures bounded local visual pixels after confirmation and zeroizes release', async () => {
    const element = appendVisual(5000, 4000);
    const drawImage = vi.fn();
    const target = {
      width: 0,
      height: 0,
      getContext: () => ({ drawImage }),
      toBlob(callback: (blob: Blob) => void) {
        callback(new Blob([new Uint8Array([7, 8, 9])], { type: 'image/png' }));
      },
    };
    const nativeCreate = document.createElement.bind(document);
    vi.spyOn(document, 'createElement').mockImplementation(((tag: string) =>
      tag === 'canvas'
        ? (target as unknown as HTMLCanvasElement)
        : nativeCreate(tag)) as typeof document.createElement);
    const result = await captureEpubRegion(
      {
        sectionId: 'spine-0',
        cfi: 'epubcfi(/6/2!/4/2)',
        element,
        rect,
      },
      () => true,
    );
    expect(drawImage).toHaveBeenCalled();
    expect(
      (result?.capture?.width ?? 0) * (result?.capture?.height ?? 0),
    ).toBeLessThanOrEqual(2_000_000);
    const bytes = result!.capture!.bytes;
    result!.capture!.release();
    result!.capture!.release();
    expect([...bytes]).toEqual([0, 0, 0]);
  });

  it('does not create a canvas when confirmation resolves after cancellation', async () => {
    const element = appendVisual();
    const confirmation = deferred<boolean>();
    const controller = new AbortController();
    const create = vi.spyOn(document, 'createElement');
    const pending = captureEpubRegion(
      {
        sectionId: 'spine-0',
        cfi: 'epubcfi(/6/2!/4/2)',
        element,
        rect,
      },
      () => confirmation.promise,
      controller.signal,
    );
    controller.abort();
    confirmation.resolve(true);
    await expect(pending).rejects.toMatchObject({ name: 'AbortError' });
    expect(create.mock.calls.filter(([tag]) => tag === 'canvas')).toHaveLength(
      0,
    );
  });
});

function appendElement(html: string): HTMLElement {
  const host = document.createElement('div');
  host.innerHTML = html;
  const element = host.firstElementChild as HTMLElement;
  document.body.append(element);
  setBounds(element, { left: 0, top: 0, width: 500, height: 200 });
  return element;
}

function appendVisual(width = 500, height = 300): HTMLElement {
  const element = appendElement(
    '<figure><img src="data:image/png;base64,iVBORw0KGgo=" alt="diagram"></figure>',
  );
  setBounds(element, { left: 0, top: 0, width, height });
  const image = element.querySelector('img')!;
  setBounds(image, { left: 0, top: 0, width, height });
  Object.defineProperties(image, {
    naturalWidth: { value: width },
    naturalHeight: { value: height },
  });
  return element;
}

function setBounds(
  element: Element,
  bounds: { left: number; top: number; width: number; height: number },
) {
  element.getBoundingClientRect = () =>
    ({
      ...bounds,
      right: bounds.left + bounds.width,
      bottom: bounds.top + bounds.height,
      x: bounds.left,
      y: bounds.top,
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
