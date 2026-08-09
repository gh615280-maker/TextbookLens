import type {
  ContentAnchor,
  NormalizedRect,
  TextQuote,
  RegionAnchor,
} from '../../../lib/generated/document';
import type { PdfTextItem } from '../../import/parsers/pdf-layout';
import { assessPdfPageQuality } from '../../indexing/pdf-quality';

export const PDF_REGION_CAPTURE_LIMITS = Object.freeze({
  maxScale: 2,
  maxWidth: 2048,
  maxHeight: 2048,
  maxPixels: 2_000_000,
  maxBytes: 4 * 1024 * 1024,
});

export interface PdfViewportLike {
  width: number;
  height: number;
  convertToViewportPoint?(x: number, y: number): number[];
  convertToViewportRectangle?(rect: [number, number, number, number]): number[];
}

export interface PdfRegionPageData {
  page: number;
  rect: NormalizedRect;
  anchorRect?: NormalizedRect;
  pageElement?: HTMLElement;
  viewport: PdfViewportLike;
  textItems: readonly PdfTextItem[];
  canvas: HTMLCanvasElement | null;
}

export interface PdfRegionCapture {
  text: string | null;
  anchor: ContentAnchor;
  capture: {
    mimeType: 'image/png';
    width: number;
    height: number;
    bytes: Uint8Array;
    release(): void;
  } | null;
}

export async function capturePdfRegion(
  data: PdfRegionPageData,
  confirmVisualCapture: () => boolean | Promise<boolean>,
  signal?: AbortSignal,
): Promise<PdfRegionCapture | null> {
  throwIfAborted(signal);
  const items = intersectingItems(data);
  const pageQuality = assessPdfPageQuality({
    pageNumber: data.page,
    width: data.viewport.width,
    height: data.viewport.height,
    items: data.textItems,
  });
  const text = normalizeReadingOrder(items);
  if (pageQuality.qualityReason === 'reliable_text' && text) {
    const quote = textQuote(text);
    const normalizedBytes = utf8(text);
    let hash: string;
    try {
      hash = await sha256(normalizedBytes);
    } finally {
      normalizedBytes.fill(0);
    }
    return {
      text,
      anchor: regionAnchor(
        data.page,
        data.anchorRect ?? data.rect,
        hash,
        quote,
      ),
      capture: null,
    };
  }
  if (!(await confirmVisualCapture())) return null;
  throwIfAborted(signal);
  if (!data.canvas) return null;
  const capture = await cropPng(
    data.canvas,
    data.rect,
    signal,
    data.pageElement,
  );
  try {
    const hash = await sha256(capture.bytes);
    throwIfAborted(signal);
    return {
      text: text || null,
      anchor: regionAnchor(
        data.page,
        data.anchorRect ?? data.rect,
        hash,
        text ? textQuote(text) : null,
      ),
      capture,
    };
  } catch (error) {
    capture.release();
    throw error;
  }
}

/** Verifies the recorded page/rectangle content without nearest-page relocation. */
export async function verifyPdfRegionAnchor(
  page: HTMLElement,
  anchor: RegionAnchor,
  signal?: AbortSignal,
): Promise<boolean> {
  if (
    anchor.locator.format !== 'pdf' ||
    !/^[0-9a-f]{64}$/u.test(anchor.contentSha256) ||
    !validRect(anchor.rect) ||
    page.dataset.pageNumber !== String(anchor.locator.page)
  ) {
    return false;
  }
  throwIfAborted(signal);
  if (anchor.textFallback) {
    const bytes = utf8(anchor.textFallback.exact);
    let textHash: string;
    try {
      textHash = await sha256(bytes);
    } finally {
      bytes.fill(0);
    }
    if (textHash === anchor.contentSha256) {
      const text = (page.textContent ?? '')
        .normalize('NFC')
        .replace(/\s+/gu, ' ')
        .trim();
      return occurrences(text, anchor.textFallback.exact) === 1;
    }
  }
  const canvas = page.querySelector<HTMLCanvasElement>('canvas');
  if (!canvas) return false;
  try {
    const capture = await cropPng(canvas, anchor.rect, signal, page);
    try {
      return (await sha256(capture.bytes)) === anchor.contentSha256;
    } finally {
      capture.release();
    }
  } catch {
    return false;
  }
}

/** Distinguishes a pixel-backed region from a text-backed region without retaining pixels. */
export async function isVisualPdfRegionAnchor(
  anchor: RegionAnchor,
): Promise<boolean> {
  if (!/^[0-9a-f]{64}$/u.test(anchor.contentSha256)) return false;
  if (!anchor.textFallback) return true;
  const bytes = utf8(anchor.textFallback.exact);
  try {
    return (await sha256(bytes)) !== anchor.contentSha256;
  } finally {
    bytes.fill(0);
  }
}

function intersectingItems(data: PdfRegionPageData): PdfTextItem[] {
  const region = {
    left: data.rect.x * data.viewport.width,
    top: data.rect.y * data.viewport.height,
    right: (data.rect.x + data.rect.width) * data.viewport.width,
    bottom: (data.rect.y + data.rect.height) * data.viewport.height,
  };
  return data.textItems.filter((item) => {
    const height = Math.hypot(item.transform[2] ?? 0, item.transform[3] ?? 0);
    const viewportRect = viewportRectangle(data.viewport, [
      item.transform[4],
      item.transform[5],
      item.transform[4] + item.width,
      item.transform[5] + height,
    ]);
    const bounds = {
      left: Math.min(viewportRect[0], viewportRect[2]),
      top: Math.min(viewportRect[1], viewportRect[3]),
      right: Math.max(viewportRect[0], viewportRect[2]),
      bottom: Math.max(viewportRect[1], viewportRect[3]),
    };
    return (
      bounds.right > region.left &&
      bounds.left < region.right &&
      bounds.bottom > region.top &&
      bounds.top < region.bottom
    );
  });
}

function viewportRectangle(
  viewport: PdfViewportLike,
  rect: [number, number, number, number],
) {
  if (viewport.convertToViewportRectangle) {
    return viewport.convertToViewportRectangle(rect);
  }
  if (viewport.convertToViewportPoint) {
    const start = viewport.convertToViewportPoint(rect[0], rect[1]);
    const end = viewport.convertToViewportPoint(rect[2], rect[3]);
    return [start[0], start[1], end[0], end[1]];
  }
  throw new Error('PDF_REGION_VIEWPORT_UNSUPPORTED');
}

function normalizeReadingOrder(items: readonly PdfTextItem[]): string {
  return [...items]
    .sort(
      (a, b) =>
        b.transform[5] - a.transform[5] ||
        a.transform[4] - b.transform[4] ||
        a.str.localeCompare(b.str),
    )
    .map((item) => item.str.normalize('NFC').replace(/\s+/gu, ' ').trim())
    .filter(Boolean)
    .join(' ')
    .replace(/\s+/gu, ' ')
    .trim();
}

async function cropPng(
  canvas: HTMLCanvasElement,
  rect: NormalizedRect,
  signal?: AbortSignal,
  pageElement?: HTMLElement,
) {
  const source = canvasSourceRect(canvas, rect, pageElement);
  const sourceWidth = source.width;
  const sourceHeight = source.height;
  const scale = Math.min(
    PDF_REGION_CAPTURE_LIMITS.maxScale,
    PDF_REGION_CAPTURE_LIMITS.maxWidth / sourceWidth,
    PDF_REGION_CAPTURE_LIMITS.maxHeight / sourceHeight,
    Math.sqrt(
      PDF_REGION_CAPTURE_LIMITS.maxPixels / (sourceWidth * sourceHeight),
    ),
  );
  const width = Math.max(1, Math.floor(sourceWidth * Math.min(1, scale)));
  const height = Math.max(1, Math.floor(sourceHeight * Math.min(1, scale)));
  const target = document.createElement('canvas');
  target.width = width;
  target.height = height;
  try {
    const context = target.getContext('2d', { alpha: false });
    if (!context) throw new Error('PDF_REGION_CAPTURE_FAILED');
    context.drawImage(
      canvas,
      source.x,
      source.y,
      sourceWidth,
      sourceHeight,
      0,
      0,
      width,
      height,
    );
    const blob = await new Promise<Blob | null>((resolve) =>
      target.toBlob(resolve, 'image/png'),
    );
    throwIfAborted(signal);
    if (
      !blob ||
      blob.type !== 'image/png' ||
      blob.size > PDF_REGION_CAPTURE_LIMITS.maxBytes
    )
      throw new Error('PDF_REGION_CAPTURE_LIMIT_EXCEEDED');
    const bytes = new Uint8Array(await blob.arrayBuffer());
    try {
      throwIfAborted(signal);
      let released = false;
      return {
        mimeType: 'image/png' as const,
        width,
        height,
        bytes,
        release() {
          if (released) return;
          released = true;
          bytes.fill(0);
        },
      };
    } catch (error) {
      bytes.fill(0);
      throw error;
    }
  } finally {
    target.width = 0;
    target.height = 0;
  }
}

function canvasSourceRect(
  canvas: HTMLCanvasElement,
  rect: NormalizedRect,
  pageElement?: HTMLElement,
) {
  const pageBounds = pageElement?.getBoundingClientRect();
  const canvasBounds = canvas.getBoundingClientRect();
  if (
    pageBounds &&
    pageBounds.width > 0 &&
    pageBounds.height > 0 &&
    canvasBounds.width > 0 &&
    canvasBounds.height > 0
  ) {
    const selectedLeft = pageBounds.left + rect.x * pageBounds.width;
    const selectedTop = pageBounds.top + rect.y * pageBounds.height;
    const selectedRight = selectedLeft + rect.width * pageBounds.width;
    const selectedBottom = selectedTop + rect.height * pageBounds.height;
    const left = Math.max(canvasBounds.left, selectedLeft);
    const top = Math.max(canvasBounds.top, selectedTop);
    const right = Math.min(canvasBounds.right, selectedRight);
    const bottom = Math.min(canvasBounds.bottom, selectedBottom);
    if (right > left && bottom > top) {
      const x = Math.max(
        0,
        Math.floor(
          ((left - canvasBounds.left) / canvasBounds.width) * canvas.width,
        ),
      );
      const y = Math.max(
        0,
        Math.floor(
          ((top - canvasBounds.top) / canvasBounds.height) * canvas.height,
        ),
      );
      const rightPixel = Math.min(
        canvas.width,
        Math.ceil(
          ((right - canvasBounds.left) / canvasBounds.width) * canvas.width,
        ),
      );
      const bottomPixel = Math.min(
        canvas.height,
        Math.ceil(
          ((bottom - canvasBounds.top) / canvasBounds.height) * canvas.height,
        ),
      );
      return {
        x,
        y,
        width: Math.max(1, rightPixel - x),
        height: Math.max(1, bottomPixel - y),
      };
    }
  }
  return {
    x: Math.floor(rect.x * canvas.width),
    y: Math.floor(rect.y * canvas.height),
    width: Math.max(1, Math.floor(rect.width * canvas.width)),
    height: Math.max(1, Math.floor(rect.height * canvas.height)),
  };
}

function regionAnchor(
  page: number,
  rect: NormalizedRect,
  contentSha256: string,
  textFallback: TextQuote | null,
): ContentAnchor {
  return {
    kind: 'region',
    region: {
      locator: { format: 'pdf', page },
      rect,
      contentSha256,
      textFallback,
    },
  };
}

function textQuote(exact: string): TextQuote {
  return { exact, prefix: '', suffix: '' };
}
function validRect(rect: NormalizedRect): boolean {
  return (
    [rect.x, rect.y, rect.width, rect.height].every(Number.isFinite) &&
    rect.x >= 0 &&
    rect.y >= 0 &&
    rect.width > 0 &&
    rect.height > 0 &&
    rect.x + rect.width <= 1 &&
    rect.y + rect.height <= 1
  );
}
function occurrences(value: string, needle: string): number {
  if (!needle) return 0;
  let count = 0;
  let index = 0;
  while ((index = value.indexOf(needle, index)) >= 0) {
    count += 1;
    if (count > 1) return count;
    index += needle.length;
  }
  return count;
}
function utf8(text: string) {
  return new TextEncoder().encode(
    text.normalize('NFC').replace(/\s+/gu, ' ').trim(),
  );
}
async function sha256(bytes: Uint8Array) {
  const copy = Uint8Array.from(bytes);
  try {
    const digest = await crypto.subtle.digest('SHA-256', copy.buffer);
    return [...new Uint8Array(digest)]
      .map((value) => value.toString(16).padStart(2, '0'))
      .join('');
  } finally {
    copy.fill(0);
  }
}
function throwIfAborted(signal?: AbortSignal) {
  if (signal?.aborted) throw new DOMException('Aborted', 'AbortError');
}
