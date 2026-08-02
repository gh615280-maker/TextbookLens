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
});
