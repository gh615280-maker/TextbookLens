import { describe, expect, it, vi } from 'vitest';

import type {
  AnnotationMarker,
  MarkerRelocation,
  ReaderAdapter,
} from '../contracts';
import { MarkerLayer } from './MarkerLayer';

const pdfAnchor = {
  kind: 'text' as const,
  selection: {
    locator: {
      format: 'pdf' as const,
      startPage: 1,
      endPage: 1,
      rectsByPage: null,
    },
    quote: { exact: 'target', prefix: '', suffix: '' },
    sectionId: null,
  },
};
const markers: AnnotationMarker[] = [
  {
    id: 'ai',
    kind: 'ai_conversation',
    sequence: 1,
    summaryText: 'A short AI-generated description.',
    revision: 1,
    label: '查看 AI 对话标记',
    anchor: pdfAnchor,
    relocationStatus: 'primary',
  },
  {
    id: 'note',
    kind: 'note',
    label: '查看个人批注',
    anchor: pdfAnchor,
    relocationStatus: 'primary',
  },
  {
    id: 'lost',
    kind: 'note',
    label: '查看个人批注',
    relocationStatus: 'unresolved',
  },
];

function adapter(
  showAnnotations: ReaderAdapter['showAnnotations'],
): ReaderAdapter {
  return {
    format: 'pdf',
    open: vi.fn(),
    getSelectionSnapshot: () => null,
    navigate: vi.fn(),
    showAnnotations,
    search: vi.fn(),
    getProgress: () => ({ fraction: 0, locator: null }),
    dispose: vi.fn(),
  };
}

describe('MarkerLayer', () => {
  it('uses question numbers to navigate and saves editable descriptions', async () => {
    const root = document.createElement('aside');
    const update = vi.fn(async () => {});
    const currentAdapter = adapter(
      vi.fn(async () => [
        { annotationId: 'ai', relocationStatus: 'primary' as const },
      ]),
    );
    const layer = new MarkerLayer(root, vi.fn(), update);
    await layer.show(currentAdapter, markers.slice(0, 1));

    const locate = root.querySelector<HTMLButtonElement>(
      '[aria-label="Locate question 1 in the textbook"]',
    )!;
    expect(locate).toHaveTextContent('1');
    locate.click();
    expect(currentAdapter.navigate).toHaveBeenCalledWith(
      pdfAnchor.selection.locator,
    );

    Array.from(root.querySelectorAll('button'))
      .find((button) => button.textContent === 'Edit description')!
      .click();
    const input = root.querySelector<HTMLInputElement>('input')!;
    input.value = 'User-edited description';
    input.form!.dispatchEvent(
      new Event('submit', { bubbles: true, cancelable: true }),
    );
    await vi.waitFor(() =>
      expect(update).toHaveBeenCalledWith(
        markers[0],
        'User-edited description',
      ),
    );
    expect(root).toHaveTextContent('User-edited description');
  });

  it('rerenders existing question controls when the interface language changes', async () => {
    const root = document.createElement('aside');
    const layer = new MarkerLayer(root);
    await layer.show(
      adapter(async () => [
        { annotationId: 'ai', relocationStatus: 'primary' as const },
      ]),
      markers.slice(0, 1),
    );

    layer.setLabels({
      locate: (sequence) => `定位问题 ${sequence} 的原文位置`,
      open: '打开回答',
      edit: '编辑简述',
      save: '保存',
      cancel: '取消',
      description: (sequence) => `问题 ${sequence} 的简短说明`,
      unresolved: '无法精确恢复原文位置。',
      saveFailed: '无法保存简述。',
    });

    expect(root).toHaveTextContent('打开回答');
    expect(root).toHaveTextContent('编辑简述');
    expect(
      root.querySelector('[aria-label="定位问题 1 的原文位置"]'),
    ).not.toBeNull();
  });

  it('keeps unresolved markers in history while attaching only resolvable markers and cleans listeners', async () => {
    const root = document.createElement('aside');
    const activate = vi.fn();
    const show = vi.fn(async (): Promise<MarkerRelocation[]> => [
      { annotationId: 'ai', relocationStatus: 'primary' },
      { annotationId: 'note', relocationStatus: 'fallback' },
    ]);
    const layer = new MarkerLayer(root, activate);

    const statuses = await layer.show(adapter(show), markers);

    expect(show).toHaveBeenCalledWith(markers.slice(0, 2));
    expect(statuses).toEqual(
      expect.arrayContaining([
        { annotationId: 'lost', relocationStatus: 'unresolved' },
        { annotationId: 'note', relocationStatus: 'fallback' },
      ]),
    );
    const unresolved = root.querySelector<HTMLElement>(
      '[data-annotation-id="lost"]',
    )!;
    expect(unresolved).toHaveAttribute('data-relocation-status', 'unresolved');
    expect(
      unresolved.querySelector('[data-marker-shape="note"]'),
    ).not.toBeNull();
    unresolved.querySelector('button')!.click();
    expect(activate).toHaveBeenCalledWith([markers[2]]);
    layer.dispose();
    expect(root).toBeEmptyDOMElement();
    expect(show).toHaveBeenLastCalledWith([]);
  });

  it('discards a late adapter response after a book switch', async () => {
    const root = document.createElement('aside');
    let finish!: (value: MarkerRelocation[]) => void;
    const firstShow = vi.fn((items: AnnotationMarker[]) =>
      items.length === 0
        ? Promise.resolve([])
        : new Promise<MarkerRelocation[]>((resolve) => {
            finish = resolve;
          }),
    );
    const secondShow = vi.fn(async () => [
      { annotationId: 'note', relocationStatus: 'primary' as const },
    ]);
    const layer = new MarkerLayer(root, vi.fn());
    const first = layer.show(adapter(firstShow), markers.slice(0, 1));
    await layer.show(adapter(secondShow), markers.slice(1, 2));
    finish([{ annotationId: 'ai', relocationStatus: 'primary' }]);
    expect(await first).toEqual([]);
    expect(firstShow).toHaveBeenLastCalledWith([]);
    expect(root.querySelector('[data-annotation-id="ai"]')).toBeNull();
    expect(root.querySelector('[data-annotation-id="note"]')).not.toBeNull();
  });

  it('serializes refreshes on one adapter so an older render cannot overwrite the latest markers', async () => {
    let finish!: (value: MarkerRelocation[]) => void;
    const show = vi.fn((items: readonly AnnotationMarker[]) => {
      if (items[0]?.id === 'ai') {
        return new Promise<MarkerRelocation[]>((resolve) => {
          finish = resolve;
        });
      }
      return Promise.resolve(
        items.map((item) => ({
          annotationId: item.id,
          relocationStatus: 'primary' as const,
        })),
      );
    });
    const currentAdapter = adapter(show);
    const layer = new MarkerLayer(document.createElement('aside'), vi.fn());

    const older = layer.show(currentAdapter, markers.slice(0, 1));
    const latest = layer.show(currentAdapter, markers.slice(1, 2));
    expect(show).toHaveBeenCalledTimes(1);
    finish([{ annotationId: 'ai', relocationStatus: 'primary' }]);

    expect(await older).toEqual([]);
    expect(await latest).toEqual([
      { annotationId: 'note', relocationStatus: 'primary' },
    ]);
    expect(
      show.mock.calls.map(([items]) => items.map((item) => item.id)),
    ).toEqual([['ai'], [], ['note']]);
  });
});
