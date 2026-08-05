import type { PanelGeometry } from '../../lib/generated/panel';

export const MIN_PANEL_WIDTH = 320;
export const MIN_PANEL_HEIGHT = 240;
export const MAX_PANEL_DIMENSION = 8192;
export const DEFAULT_PANEL_GEOMETRY: Readonly<PanelGeometry> = Object.freeze({
  xRatio: 0.5,
  yRatio: 0.5,
  widthPx: 400,
  heightPx: 520,
});

export type ResizeHandle = 'n' | 'ne' | 'e' | 'se' | 's' | 'sw' | 'w' | 'nw';

export interface PanelViewport {
  readonly width: number;
  readonly height: number;
}

export interface PanelRect {
  readonly x: number;
  readonly y: number;
  readonly width: number;
  readonly height: number;
}

const SNAP_PX = 16;
const PREFERENCE_KEY = 'textbooklens.panel-geometry.v1';

export interface PanelGeometryPreference {
  readonly geometry: Readonly<PanelGeometry>;
  readonly revision: number;
}

export interface PanelGeometryPreferenceApi {
  read(): Promise<PanelGeometryPreference>;
  compareAndSet(
    expectedRevision: number,
    geometry: Readonly<PanelGeometry>,
  ): Promise<PanelGeometryPreference | null>;
}

/** Keeps a single latest layout preference without retaining an open-panel list. */
export class DebouncedPanelGeometryWriter {
  private revision: number | null = null;
  private latest: Readonly<PanelGeometry> | null = null;
  private timer: ReturnType<typeof setTimeout> | null = null;

  constructor(
    private readonly api: PanelGeometryPreferenceApi,
    private readonly delayMs = 200,
  ) {}

  save(geometry: Readonly<PanelGeometry>) {
    this.latest = normalizePanelGeometry(geometry);
    if (this.timer) clearTimeout(this.timer);
    this.timer = setTimeout(() => {
      this.timer = null;
      void this.flush();
    }, this.delayMs);
  }

  dispose() {
    if (this.timer) clearTimeout(this.timer);
    this.timer = null;
    this.latest = null;
  }

  private async flush() {
    const geometry = this.latest;
    if (!geometry) return;
    const current = await this.api.read();
    const expected = this.revision ?? current.revision;
    const saved = await this.api.compareAndSet(expected, geometry);
    if (saved) {
      this.revision = saved.revision;
      if (this.latest === geometry) this.latest = null;
      return;
    }
    // Another window won. Keep only the newest local preference and retry it.
    this.revision = null;
    if (this.latest) this.save(this.latest);
  }
}

export function localPanelGeometryPreference(): PanelGeometryPreferenceApi {
  return {
    async read() {
      return readLocalPreference();
    },
    async compareAndSet(expectedRevision, geometry) {
      const current = readLocalPreference();
      if (current.revision !== expectedRevision || !isPanelGeometry(geometry)) {
        return null;
      }
      const next: PanelGeometryPreference = {
        geometry: normalizePanelGeometry(geometry),
        revision: current.revision + 1,
      };
      try {
        window.localStorage.setItem(PREFERENCE_KEY, JSON.stringify(next));
      } catch {
        return null;
      }
      return next;
    },
  };
}

export function isPanelGeometry(value: unknown): value is PanelGeometry {
  if (!value || typeof value !== 'object') return false;
  const geometry = value as PanelGeometry;
  return (
    [
      geometry.xRatio,
      geometry.yRatio,
      geometry.widthPx,
      geometry.heightPx,
    ].every(Number.isFinite) &&
    geometry.xRatio >= 0 &&
    geometry.xRatio <= 1 &&
    geometry.yRatio >= 0 &&
    geometry.yRatio <= 1 &&
    geometry.widthPx >= MIN_PANEL_WIDTH &&
    geometry.widthPx <= MAX_PANEL_DIMENSION &&
    geometry.heightPx >= MIN_PANEL_HEIGHT &&
    geometry.heightPx <= MAX_PANEL_DIMENSION
  );
}

export function normalizePanelGeometry(
  value: unknown,
): Readonly<PanelGeometry> {
  return isPanelGeometry(value)
    ? freezeGeometry(value)
    : DEFAULT_PANEL_GEOMETRY;
}

export function panelRect(
  geometry: PanelGeometry,
  viewport: PanelViewport,
): Readonly<PanelRect> {
  const safe = normalizePanelGeometry(geometry);
  const width = Math.min(safe.widthPx, safeViewport(viewport.width));
  const height = Math.min(safe.heightPx, safeViewport(viewport.height));
  return Object.freeze({
    x: Math.round(safe.xRatio * Math.max(0, viewport.width - width)),
    y: Math.round(safe.yRatio * Math.max(0, viewport.height - height)),
    width,
    height,
  });
}

export function movePanel(
  geometry: PanelGeometry,
  delta: { x: number; y: number },
  viewport: PanelViewport,
): Readonly<PanelGeometry> {
  const rect = panelRect(geometry, viewport);
  return geometryFromRect(
    {
      ...rect,
      x: snap(rect.x + finite(delta.x)),
      y: snap(rect.y + finite(delta.y)),
    },
    viewport,
    geometry,
  );
}

export function resizePanel(
  geometry: PanelGeometry,
  handle: ResizeHandle,
  delta: { x: number; y: number },
  viewport: PanelViewport,
): Readonly<PanelGeometry> {
  const rect = panelRect(geometry, viewport);
  const dx = finite(delta.x);
  const dy = finite(delta.y);
  let { x, y, width, height } = rect;
  if (handle.includes('e')) width += dx;
  if (handle.includes('s')) height += dy;
  if (handle.includes('w')) {
    x += dx;
    width -= dx;
  }
  if (handle.includes('n')) {
    y += dy;
    height -= dy;
  }
  width = Math.max(MIN_PANEL_WIDTH, width);
  height = Math.max(MIN_PANEL_HEIGHT, height);
  if (handle.includes('w')) x = rect.x + rect.width - width;
  if (handle.includes('n')) y = rect.y + rect.height - height;
  return geometryFromRect(
    { x: snap(x), y: snap(y), width: snap(width), height: snap(height) },
    viewport,
    geometry,
  );
}

function geometryFromRect(
  rect: PanelRect,
  viewport: PanelViewport,
  previous: PanelGeometry,
): Readonly<PanelGeometry> {
  const width = clamp(rect.width, MIN_PANEL_WIDTH, MAX_PANEL_DIMENSION);
  const height = clamp(rect.height, MIN_PANEL_HEIGHT, MAX_PANEL_DIMENSION);
  const maxX = Math.max(0, viewport.width - Math.min(width, viewport.width));
  const maxY = Math.max(0, viewport.height - Math.min(height, viewport.height));
  const x = snapPosition(clamp(rect.x, 0, maxX), maxX);
  const y = snapPosition(clamp(rect.y, 0, maxY), maxY);
  return freezeGeometry({
    xRatio: maxX ? x / maxX : previous.xRatio,
    yRatio: maxY ? y / maxY : previous.yRatio,
    widthPx: width,
    heightPx: height,
  });
}

function freezeGeometry(value: PanelGeometry): Readonly<PanelGeometry> {
  return Object.freeze({
    xRatio: value.xRatio,
    yRatio: value.yRatio,
    widthPx: value.widthPx,
    heightPx: value.heightPx,
  });
}

function safeViewport(value: number): number {
  return Number.isFinite(value) && value > 0 ? value : 1;
}

function finite(value: number): number {
  return Number.isFinite(value) ? value : 0;
}

function clamp(value: number, minimum: number, maximum: number): number {
  return Math.max(minimum, Math.min(value, maximum));
}

function snap(value: number): number {
  return Math.abs(value) <= SNAP_PX ? 0 : value;
}

function snapPosition(value: number, maximum: number): number {
  if (value <= SNAP_PX) return 0;
  if (maximum - value <= SNAP_PX) return maximum;
  return value;
}

function readLocalPreference(): PanelGeometryPreference {
  try {
    const parsed: unknown = JSON.parse(
      window.localStorage.getItem(PREFERENCE_KEY) ?? 'null',
    );
    if (
      parsed &&
      typeof parsed === 'object' &&
      Number.isSafeInteger((parsed as { revision?: unknown }).revision) &&
      (parsed as { revision: number }).revision >= 0 &&
      isPanelGeometry((parsed as { geometry?: unknown }).geometry)
    ) {
      return Object.freeze({
        revision: (parsed as { revision: number }).revision,
        geometry: normalizePanelGeometry(
          (parsed as { geometry: PanelGeometry }).geometry,
        ),
      });
    }
  } catch {
    // A corrupt preference is equivalent to no preference.
  }
  return Object.freeze({ geometry: DEFAULT_PANEL_GEOMETRY, revision: 0 });
}
