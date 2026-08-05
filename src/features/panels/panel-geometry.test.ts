import { describe, expect, it, vi } from 'vitest';

import {
  DebouncedPanelGeometryWriter,
  DEFAULT_PANEL_GEOMETRY,
  isPanelGeometry,
  movePanel,
  panelRect,
  resizePanel,
} from './panel-geometry';

describe('panel geometry', () => {
  const viewport = { width: 900, height: 700 };

  it('clamps drag and all resize handles to the display and minimum size', () => {
    const moved = movePanel(
      DEFAULT_PANEL_GEOMETRY,
      { x: -1000, y: -1000 },
      viewport,
    );
    expect(panelRect(moved, viewport)).toMatchObject({ x: 0, y: 0 });
    for (const handle of [
      'n',
      'ne',
      'e',
      'se',
      's',
      'sw',
      'w',
      'nw',
    ] as const) {
      const resized = resizePanel(
        DEFAULT_PANEL_GEOMETRY,
        handle,
        { x: -5000, y: -5000 },
        viewport,
      );
      expect(isPanelGeometry(resized)).toBe(true);
      expect(panelRect(resized, viewport).x).toBeGreaterThanOrEqual(0);
      expect(panelRect(resized, viewport).y).toBeGreaterThanOrEqual(0);
    }
  });

  it('rejects non-finite values and only persists the latest debounced geometry', async () => {
    expect(
      isPanelGeometry({ ...DEFAULT_PANEL_GEOMETRY, xRatio: Number.NaN }),
    ).toBe(false);
    vi.useFakeTimers();
    const compareAndSet = vi.fn(async (_revision: number, geometry) => ({
      geometry,
      revision: 1,
    }));
    const writer = new DebouncedPanelGeometryWriter({
      read: vi.fn(async () => ({
        geometry: DEFAULT_PANEL_GEOMETRY,
        revision: 0,
      })),
      compareAndSet,
    });
    writer.save(DEFAULT_PANEL_GEOMETRY);
    writer.save({ ...DEFAULT_PANEL_GEOMETRY, widthPx: 480 });
    await vi.advanceTimersByTimeAsync(250);
    expect(compareAndSet).toHaveBeenCalledOnce();
    expect(compareAndSet).toHaveBeenLastCalledWith(
      0,
      expect.objectContaining({ widthPx: 480 }),
    );
    writer.dispose();
    vi.useRealTimers();
  });
});
