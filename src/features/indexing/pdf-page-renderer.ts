import { getDocument } from 'pdfjs-dist/legacy/build/pdf.mjs';

import type { RenderedPdfPageDto } from './indexing-contract';

export const PDF_PAGE_RENDER_VERSION = 1 as const;
export const PDF_PAGE_RENDER_SCALE = 1.5;
export const PDF_PAGE_RENDER_BACKGROUND = '#ffffff';

const APPLICATION_MAX_DIMENSION = 4096;
const APPLICATION_MAX_DECODED_PIXELS = 8_847_360;
const APPLICATION_MAX_ENCODED_BYTES = 4 * 1024 * 1024;
const APPLICATION_MAX_TOTAL_ENCODED_BYTES = 12 * 1024 * 1024;
const MAX_RENDER_ATTEMPTS = 5;

export interface PdfPageRenderLimits {
  readonly maxDimension?: number;
  readonly maxDecodedPixels?: number;
  readonly maxEncodedBytes?: number;
  readonly maxTotalEncodedBytes?: number;
  readonly alreadyEncodedBytes?: number;
}

export class PdfPageRenderError extends Error {
  constructor(
    readonly code: 'RENDER_LIMIT_EXCEEDED' | 'RENDER_FAILED',
    cause?: unknown,
  ) {
    super(code, { cause });
    this.name = 'PdfPageRenderError';
  }
}

/** Renders an in-memory local PDF source only; it never accepts a path or URL. */
export async function renderPdfPageLocally(
  source: ArrayBuffer,
  pageNumber: number,
  limits: PdfPageRenderLimits = {},
): Promise<RenderedPdfPageDto> {
  const bounded = normalizeLimits(limits);
  if (!Number.isInteger(pageNumber) || pageNumber < 1)
    throw new PdfPageRenderError('RENDER_FAILED');
  const availableTotal =
    bounded.maxTotalEncodedBytes - bounded.alreadyEncodedBytes;
  if (availableTotal <= 0)
    throw new PdfPageRenderError('RENDER_LIMIT_EXCEEDED');
  const encodedLimit = Math.min(bounded.maxEncodedBytes, availableTotal);
  const loadingTask = getDocument({
    data: new Uint8Array(source),
    isEvalSupported: false,
  } as never);
  let document: Awaited<typeof loadingTask.promise> | undefined;
  try {
    document = await loadingTask.promise;
    const page = await document.getPage(pageNumber);
    try {
      const initial = page.getViewport({
        scale: PDF_PAGE_RENDER_SCALE,
        rotation: 0,
      });
      let scale =
        PDF_PAGE_RENDER_SCALE *
        scaleForLimits(initial.width, initial.height, bounded);
      for (let attempt = 0; attempt < MAX_RENDER_ATTEMPTS; attempt += 1) {
        const viewport = page.getViewport({ scale, rotation: 0 });
        const width = Math.floor(viewport.width);
        const height = Math.floor(viewport.height);
        if (!isWithinRasterLimits(width, height, bounded))
          throw new PdfPageRenderError('RENDER_LIMIT_EXCEEDED');
        const bytes = await renderPng(
          page as unknown as RenderablePdfPage,
          viewport,
          width,
          height,
        );
        if (bytes.byteLength <= encodedLimit) {
          const sha256 = await sha256Hex(bytes);
          return {
            schemaVersion: PDF_PAGE_RENDER_VERSION,
            mimeType: 'image/png',
            width,
            height,
            decodedPixelCount: width * height,
            encodedByteLength: bytes.byteLength,
            sha256,
            bytes,
          };
        }
        bytes.fill(0);
        scale *= Math.min(
          0.9,
          Math.sqrt(encodedLimit / bytes.byteLength) * 0.95,
        );
      }
      throw new PdfPageRenderError('RENDER_LIMIT_EXCEEDED');
    } finally {
      page.cleanup();
    }
  } catch (error) {
    if (error instanceof PdfPageRenderError) throw error;
    throw new PdfPageRenderError('RENDER_FAILED', error);
  } finally {
    await document?.cleanup();
    await loadingTask.destroy();
  }
}

/** Wipes the short-lived capture after submit, cancellation, or a local error. */
export function releaseRenderedPdfPage(page: RenderedPdfPageDto): void {
  page.bytes.fill(0);
}

function normalizeLimits(limits: PdfPageRenderLimits) {
  return {
    maxDimension: smallerPositive(
      limits.maxDimension,
      APPLICATION_MAX_DIMENSION,
    ),
    maxDecodedPixels: smallerPositive(
      limits.maxDecodedPixels,
      APPLICATION_MAX_DECODED_PIXELS,
    ),
    maxEncodedBytes: smallerPositive(
      limits.maxEncodedBytes,
      APPLICATION_MAX_ENCODED_BYTES,
    ),
    maxTotalEncodedBytes: smallerPositive(
      limits.maxTotalEncodedBytes,
      APPLICATION_MAX_TOTAL_ENCODED_BYTES,
    ),
    alreadyEncodedBytes: Math.max(0, limits.alreadyEncodedBytes ?? 0),
  };
}

function smallerPositive(
  value: number | undefined,
  applicationMaximum: number,
): number {
  if (value === undefined) return applicationMaximum;
  if (!Number.isFinite(value) || value <= 0)
    throw new PdfPageRenderError('RENDER_LIMIT_EXCEEDED');
  return Math.min(Math.floor(value), applicationMaximum);
}

function scaleForLimits(
  width: number,
  height: number,
  limits: ReturnType<typeof normalizeLimits>,
): number {
  if (
    !Number.isFinite(width) ||
    !Number.isFinite(height) ||
    width <= 0 ||
    height <= 0
  )
    throw new PdfPageRenderError('RENDER_FAILED');
  return Math.min(
    1,
    limits.maxDimension / width,
    limits.maxDimension / height,
    Math.sqrt(limits.maxDecodedPixels / (width * height)),
  );
}

function isWithinRasterLimits(
  width: number,
  height: number,
  limits: ReturnType<typeof normalizeLimits>,
): boolean {
  return (
    width > 0 &&
    height > 0 &&
    width <= limits.maxDimension &&
    height <= limits.maxDimension &&
    width * height <= limits.maxDecodedPixels
  );
}

interface RenderablePdfPage {
  render(options: {
    canvasContext: CanvasRenderingContext2D;
    viewport: object;
    background: string;
    intent: 'display';
  }): { promise: Promise<void> };
}

async function renderPng(
  page: RenderablePdfPage,
  viewport: object,
  width: number,
  height: number,
): Promise<Uint8Array> {
  const canvas = document.createElement('canvas');
  canvas.width = width;
  canvas.height = height;
  try {
    const context = canvas.getContext('2d', { alpha: false });
    if (!context) throw new PdfPageRenderError('RENDER_FAILED');
    await page.render({
      canvasContext: context,
      viewport,
      background: PDF_PAGE_RENDER_BACKGROUND,
      intent: 'display',
    }).promise;
    const blob = await new Promise<Blob | null>((resolve) =>
      canvas.toBlob(resolve, 'image/png'),
    );
    if (!blob || blob.type !== 'image/png')
      throw new PdfPageRenderError('RENDER_FAILED');
    const bytes = new Uint8Array(await blob.arrayBuffer());
    if (
      !isPng(bytes) ||
      pngDimensions(bytes)?.width !== width ||
      pngDimensions(bytes)?.height !== height
    )
      throw new PdfPageRenderError('RENDER_FAILED');
    return bytes;
  } finally {
    canvas.width = 0;
    canvas.height = 0;
  }
}

function isPng(bytes: Uint8Array): boolean {
  return (
    bytes.length >= 24 &&
    [137, 80, 78, 71, 13, 10, 26, 10].every(
      (value, index) => bytes[index] === value,
    )
  );
}

function pngDimensions(
  bytes: Uint8Array,
): { width: number; height: number } | null {
  if (!isPng(bytes)) return null;
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  return { width: view.getUint32(16), height: view.getUint32(20) };
}

async function sha256Hex(bytes: Uint8Array): Promise<string> {
  const copy = Uint8Array.from(bytes);
  const digest = await crypto.subtle.digest(
    'SHA-256',
    copy.buffer as ArrayBuffer,
  );
  return [...new Uint8Array(digest)]
    .map((part) => part.toString(16).padStart(2, '0'))
    .join('');
}
