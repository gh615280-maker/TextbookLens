import { beforeEach, describe, expect, it, vi } from 'vitest';

import {
  captureDocxRegion,
  hashDocxRegionBlock,
  resolveDocxRegionAnchor,
} from './docx-region-capture';

const rect = { x: 0.1, y: 0.1, width: 0.8, height: 0.8 };

describe('captureDocxRegion', () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    document.body.replaceChildren();
  });

  it('returns reliable block text with no bytes or confirmation', async () => {
    const block = appendBlock(
      '<p data-block-id="stable">Reliable DOCX textbook text with enough useful characters.</p>',
    );
    const confirm = vi.fn(() => {
      throw new Error('confirmation must not run');
    });
    const result = await captureDocxRegion(
      { blockId: 'stable', block, rect },
      confirm,
    );
    expect(result).toMatchObject({
      text: 'Reliable DOCX textbook text with enough useful characters.',
      capture: null,
      anchor: {
        kind: 'region',
        region: {
          locator: { format: 'docx', blockId: 'stable' },
          rect,
          contentSha256: expect.stringMatching(/^[0-9a-f]{64}$/),
        },
      },
    });
    expect(confirm).not.toHaveBeenCalled();
    expect(JSON.stringify(result?.anchor)).not.toMatch(
      /data:image|outerHTML|ownerDocument|[a-z]:\\/iu,
    );
  });

  it('relocates an exact stable block after rerender and rejects change or ambiguity', async () => {
    const root = document.createElement('main');
    document.body.append(root);
    root.innerHTML =
      '<p data-block-id="stable" data-section-id="s" style="font-size:12px">Stable rerendered block content.</p>';
    const original = root.firstElementChild as HTMLElement;
    const hash = await hashDocxRegionBlock(original);
    const anchor = {
      locator: { format: 'docx' as const, blockId: 'stable' },
      rect,
      contentSha256: hash,
      textFallback: null,
    };
    root.innerHTML =
      '<p data-block-id="stable" data-section-id="s" style="font-size:30px">Stable rerendered block content.</p>';
    await expect(resolveDocxRegionAnchor(root, anchor)).resolves.toMatchObject({
      block: root.firstElementChild,
      rect,
    });
    root.firstElementChild!.textContent = 'Changed content.';
    await expect(resolveDocxRegionAnchor(root, anchor)).resolves.toBeNull();
    root.insertAdjacentHTML(
      'beforeend',
      '<p data-block-id="stable">Stable rerendered block content.</p>',
    );
    await expect(resolveDocxRegionAnchor(root, anchor)).resolves.toBeNull();
  });

  it('creates no image bytes when mixed/visual confirmation is rejected', async () => {
    const block = appendVisual();
    const create = vi.spyOn(document, 'createElement');
    await expect(
      captureDocxRegion({ blockId: 'visual', block, rect }, () => false),
    ).resolves.toBeNull();
    expect(create.mock.calls.filter(([tag]) => tag === 'canvas')).toHaveLength(
      0,
    );
  });

  it('bounds visual capture and wipes owned bytes on idempotent release', async () => {
    const block = appendVisual(5000, 4000);
    const drawImage = vi.fn();
    const target = {
      width: 0,
      height: 0,
      getContext: () => ({ drawImage }),
      toBlob(callback: (blob: Blob) => void) {
        callback(new Blob([new Uint8Array([1, 2, 3])], { type: 'image/png' }));
      },
    };
    const nativeCreate = document.createElement.bind(document);
    vi.spyOn(document, 'createElement').mockImplementation(((tag: string) =>
      tag === 'canvas'
        ? (target as unknown as HTMLCanvasElement)
        : nativeCreate(tag)) as typeof document.createElement);
    const result = await captureDocxRegion(
      { blockId: 'visual', block, rect },
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

  it('materializes no canvas after an aborted confirmation resolves late', async () => {
    const block = appendVisual();
    const confirmation = deferred<boolean>();
    const controller = new AbortController();
    const create = vi.spyOn(document, 'createElement');
    const pending = captureDocxRegion(
      { blockId: 'visual', block, rect },
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

function appendBlock(html: string): HTMLElement {
  const host = document.createElement('div');
  host.innerHTML = html;
  const block = host.firstElementChild as HTMLElement;
  document.body.append(block);
  setBounds(block, { left: 0, top: 0, width: 500, height: 200 });
  return block;
}

function appendVisual(width = 500, height = 300): HTMLElement {
  const block = appendBlock(
    '<p data-block-id="visual">caption<img src="data:image/png;base64,iVBORw0KGgo=" alt="diagram"></p>',
  );
  setBounds(block, { left: 0, top: 0, width, height });
  const image = block.querySelector('img')!;
  setBounds(image, { left: 0, top: 0, width, height });
  Object.defineProperties(image, {
    naturalWidth: { value: width },
    naturalHeight: { value: height },
  });
  return block;
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
