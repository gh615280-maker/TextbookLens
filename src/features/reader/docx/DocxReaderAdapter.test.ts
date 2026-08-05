import { describe, expect, it, vi } from 'vitest';
import type { ReaderAdapterEvents } from '../contracts';
import { DocxReaderAdapter } from './DocxReaderAdapter';
describe('DocxReaderAdapter', () => {
  it('renders only local sanitized content and restores block anchors', async () => {
    const events: ReaderAdapterEvents = {
      onSelection: vi.fn(),
      onProgress: vi.fn(),
      onMarkerActivate: vi.fn(),
      onFailure: vi.fn(),
    };
    const adapter = new DocxReaderAdapter(document.body, events);
    await adapter.open({
      kind: 'sanitized_html',
      html: '<p data-section-id="s" data-block-id="b" onclick="bad()">safe<script>bad()</script><img src="https://bad.test/a"></p>',
    });
    expect(document.querySelector('script')).toBeNull();
    expect(document.querySelector('img')?.hasAttribute('src')).toBe(false);
    expect(
      await adapter.navigate({
        format: 'docx',
        startBlockId: 'b',
        startOffset: 0,
        endBlockId: 'b',
        endOffset: 4,
      }),
    ).toEqual({ found: true });
    adapter.dispose();
    expect(document.body.textContent).toBe('');
  });
  it('reports deterministic primary, section-confined unique fallback, and ambiguity', async () => {
    const getClientRects = Range.prototype.getClientRects;
    Range.prototype.getClientRects = () => [] as unknown as DOMRectList;
    const failure = vi.fn();
    const adapter = new DocxReaderAdapter(document.body, {
      onSelection: vi.fn(),
      onProgress: vi.fn(),
      onMarkerActivate: vi.fn(),
      onFailure: failure,
    });
    await adapter.open({
      kind: 'sanitized_html',
      html: '<p data-section-id="s" data-block-id="actual">prefix target suffix</p><p data-section-id="other" data-block-id="other">target</p>',
    });
    const marker = {
      id: 'docx',
      kind: 'note' as const,
      label: '查看个人批注',
      relocationStatus: 'primary' as const,
      anchor: {
        locator: {
          format: 'docx' as const,
          startBlockId: 'actual',
          startOffset: 7,
          endBlockId: 'actual',
          endOffset: 13,
        },
        quote: { exact: 'target', prefix: 'prefix ', suffix: ' suffix' },
        sectionId: 's',
      },
    };
    expect(await adapter.showAnnotations([marker])).toEqual([
      { annotationId: 'docx', relocationStatus: 'primary' },
    ]);
    expect(
      await adapter.showAnnotations([
        {
          ...marker,
          anchor: {
            ...marker.anchor,
            locator: {
              ...marker.anchor.locator,
              startBlockId: 'missing',
              endBlockId: 'missing',
            },
          },
        },
      ]),
    ).toEqual([{ annotationId: 'docx', relocationStatus: 'fallback' }]);
    document
      .querySelector('[data-section-id="s"]')!
      .insertAdjacentHTML(
        'afterend',
        '<p data-section-id="s" data-block-id="duplicate">prefix target suffix</p>',
      );
    expect(
      await adapter.showAnnotations([
        {
          ...marker,
          anchor: {
            ...marker.anchor,
            locator: {
              ...marker.anchor.locator,
              startBlockId: 'missing',
              endBlockId: 'missing',
            },
          },
        },
      ]),
    ).toEqual([{ annotationId: 'docx', relocationStatus: 'unresolved' }]);
    expect(document.body.querySelector('.docx-reader-markers')).toBeNull();
    expect(failure).toHaveBeenCalledWith(
      expect.objectContaining({ code: 'ANCHOR_NOT_FOUND' }),
    );
    Range.prototype.getClientRects = getClientRects;
  });

  it('freezes one stable block region and avoids image bytes for reliable text', async () => {
    const { adapter, root } = await openRegionAdapter(
      '<p data-section-id="s" data-block-id="stable">Reliable DOCX region text with enough useful characters.</p>',
    );
    const block = root.querySelector<HTMLElement>('[data-block-id]')!;
    setBounds(block, 0, 0, 100, 100);
    const confirm = vi.fn(() => {
      throw new Error('confirmation must not run');
    });
    const pending = adapter.beginRegionSelection({
      confirmVisualCapture: confirm,
    });
    block.dispatchEvent(pointer('pointerdown', 10, 10));
    block.dispatchEvent(pointer('pointerup', 70, 70));
    await expect(pending).resolves.toMatchObject({
      blockId: 'stable',
      rect: { x: 0.1, y: 0.1, width: 0.6, height: 0.6 },
      capture: null,
      anchor: {
        kind: 'region',
        region: {
          locator: { format: 'docx', blockId: 'stable' },
          contentSha256: expect.stringMatching(/^[0-9a-f]{64}$/),
        },
      },
    });
    expect(confirm).not.toHaveBeenCalled();
    adapter.dispose();
    root.remove();
  });

  it('rejects cross-block and tiny drags without returning a viewport rect', async () => {
    const { adapter, root } = await openRegionAdapter(
      '<p data-section-id="s" data-block-id="a">First reliable block text.</p><p data-section-id="s" data-block-id="b">Second reliable block text.</p>',
    );
    const [first, second] =
      root.querySelectorAll<HTMLElement>('[data-block-id]');
    setBounds(first!, 0, 0, 100, 100);
    setBounds(second!, 0, 120, 100, 100);
    const crossBlock = adapter.beginRegionSelection({
      confirmVisualCapture: () => false,
    });
    first!.dispatchEvent(pointer('pointerdown', 10, 10));
    second!.dispatchEvent(pointer('pointerup', 50, 150));
    await expect(crossBlock).rejects.toMatchObject({
      instructionCode: 'docx_region_cross_block',
    });
    const tiny = adapter.beginRegionSelection({
      confirmVisualCapture: () => false,
    });
    first!.dispatchEvent(pointer('pointerdown', 10, 10));
    first!.dispatchEvent(pointer('pointerup', 17, 50));
    await expect(tiny).rejects.toMatchObject({
      instructionCode: 'docx_region_too_small',
    });
    adapter.dispose();
    root.remove();
  });

  it('cancels on keyboard/pointer and ignores visual confirmation after dispose', async () => {
    const text = await openRegionAdapter(
      '<p data-section-id="s" data-block-id="stable">Reliable DOCX region text.</p>',
    );
    const escape = text.adapter.beginRegionSelection({
      confirmVisualCapture: () => false,
    });
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }));
    await expect(escape).rejects.toMatchObject({
      instructionCode: 'docx_region_cancelled',
    });
    const pointerCancel = text.adapter.beginRegionSelection({
      confirmVisualCapture: () => false,
    });
    window.dispatchEvent(new Event('pointercancel'));
    await expect(pointerCancel).rejects.toMatchObject({
      instructionCode: 'docx_region_cancelled',
    });
    text.adapter.dispose();
    text.root.remove();

    const visual = await openRegionAdapter(
      '<p data-section-id="s" data-block-id="visual"><img src="data:image/png;base64,iVBORw0KGgo=" alt="diagram"></p>',
    );
    const block = visual.root.querySelector<HTMLElement>('[data-block-id]')!;
    const image = block.querySelector('img')!;
    setBounds(block, 0, 0, 100, 100);
    setBounds(image, 0, 0, 100, 100);
    Object.defineProperties(image, {
      naturalWidth: { value: 100 },
      naturalHeight: { value: 100 },
    });
    const confirmation = deferred<boolean>();
    const confirm = vi.fn(() => confirmation.promise);
    const create = vi.spyOn(document, 'createElement');
    const pending = visual.adapter.beginRegionSelection({
      confirmVisualCapture: confirm,
    });
    image.dispatchEvent(pointer('pointerdown', 10, 10));
    image.dispatchEvent(pointer('pointerup', 70, 70));
    await vi.waitFor(() => expect(confirm).toHaveBeenCalledOnce());
    visual.adapter.dispose();
    confirmation.resolve(true);
    await expect(pending).rejects.toMatchObject({
      instructionCode: 'docx_region_cancelled',
    });
    await Promise.resolve();
    expect(create.mock.calls.filter(([tag]) => tag === 'canvas')).toHaveLength(
      0,
    );
    visual.root.remove();
  });
});

async function openRegionAdapter(html: string) {
  const root = document.createElement('main');
  document.body.append(root);
  const adapter = new DocxReaderAdapter(root, {
    onSelection: vi.fn(),
    onProgress: vi.fn(),
    onMarkerActivate: vi.fn(),
    onFailure: vi.fn(),
  });
  await adapter.open({ kind: 'sanitized_html', html });
  return { adapter, root };
}

function pointer(type: string, clientX: number, clientY: number) {
  return new MouseEvent(type, {
    bubbles: true,
    button: 0,
    clientX,
    clientY,
  }) as unknown as PointerEvent;
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
