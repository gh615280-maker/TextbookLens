import type {
  ContentAnchor,
  NormalizedRect,
  RegionAnchor,
  TextQuote,
} from '../../../lib/generated/document';
import { uniqueDocxRegionBlock } from './docx-region-selection';

export const DOCX_REGION_CAPTURE_LIMITS = Object.freeze({
  maxScale: 2,
  maxWidth: 2048,
  maxHeight: 2048,
  maxPixels: 2_000_000,
  maxBytes: 4 * 1024 * 1024,
  maxTextCodePoints: 32_768,
  maxCanonicalBytes: 8 * 1024 * 1024,
  maxDomNodes: 20_000,
});

export interface DocxRegionCaptureData {
  blockId: string;
  block: HTMLElement;
  rect: NormalizedRect;
}

export interface DocxRegionCapture {
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

export async function captureDocxRegion(
  data: DocxRegionCaptureData,
  confirmVisualCapture: () => boolean | Promise<boolean>,
  signal?: AbortSignal,
): Promise<DocxRegionCapture | null> {
  throwIfAborted(signal);
  validateRect(data.rect);
  if (
    !data.blockId ||
    data.block.dataset.blockId !== data.blockId ||
    !data.block.isConnected
  )
    throw new Error('DOCX_REGION_CAPTURE_UNAVAILABLE');
  const inspection = inspectRegion(data.block, data.rect);
  const contentSha256 = await hashDocxRegionBlock(data.block, signal);
  throwIfAborted(signal);
  const fallback = quoteFor(data.block, inspection.text);
  if (inspection.reliableText) {
    return {
      text: inspection.text,
      anchor: docxRegionAnchor(data, contentSha256, fallback),
      capture: null,
    };
  }
  if (!(await confirmVisualCapture())) return null;
  throwIfAborted(signal);
  if (!data.block.isConnected)
    throw new Error('DOCX_REGION_CAPTURE_UNAVAILABLE');
  const capture = await captureDomRegion(
    data.block,
    data.rect,
    inspection.hasMixedContent,
    signal,
  );
  try {
    throwIfAborted(signal);
    return {
      text: inspection.text,
      anchor: docxRegionAnchor(data, contentSha256, fallback),
      capture,
    };
  } catch (error) {
    capture.release();
    throw error;
  }
}

/** Exact block lookup plus hash verification; no quote or nearest-block fallback. */
export async function resolveDocxRegionAnchor(
  root: HTMLElement,
  anchor: RegionAnchor,
  signal?: AbortSignal,
): Promise<{ block: HTMLElement; rect: NormalizedRect } | null> {
  const locator = anchor.locator;
  if (
    locator.format !== 'docx' ||
    !LOWER_SHA256.test(anchor.contentSha256) ||
    !isValidRect(anchor.rect)
  )
    return null;
  const block = uniqueDocxRegionBlock(root, locator.blockId);
  if (!block) return null;
  const hash = await hashDocxRegionBlock(block, signal);
  return constantTimeEqual(hash, anchor.contentSha256)
    ? { block, rect: { ...anchor.rect } }
    : null;
}

/** Stable across rerendered markup styles and viewport geometry. */
export async function hashDocxRegionBlock(
  block: Element,
  signal?: AbortSignal,
): Promise<string> {
  const canonical = canonicalBlock(block);
  const bytes = new TextEncoder().encode(canonical);
  if (bytes.byteLength > DOCX_REGION_CAPTURE_LIMITS.maxCanonicalBytes) {
    bytes.fill(0);
    throw new Error('DOCX_REGION_CAPTURE_LIMIT_EXCEEDED');
  }
  try {
    throwIfAborted(signal);
    return await sha256(bytes);
  } finally {
    bytes.fill(0);
  }
}

function inspectRegion(block: HTMLElement, rect: NormalizedRect) {
  if (
    codePointLengthExceeds(
      block.textContent ?? '',
      DOCX_REGION_CAPTURE_LIMITS.maxTextCodePoints,
    )
  )
    throw new Error('DOCX_REGION_CAPTURE_LIMIT_EXCEEDED');
  const bounds = block.getBoundingClientRect();
  const region = denormalize(bounds, rect);
  const visual = [...block.querySelectorAll<HTMLElement>(VISUAL_SELECTOR)];
  if (block.matches(VISUAL_SELECTOR)) visual.unshift(block);
  const intersectingVisual = unique(visual).filter((candidate) =>
    candidate === block
      ? true
      : intersects(candidate.getBoundingClientRect(), region),
  );
  const textParts: string[] = [];
  let measuredText = false;
  const walker = block.ownerDocument.createTreeWalker(
    block,
    NodeFilter.SHOW_TEXT,
  );
  let node = walker.nextNode() as Text | null;
  while (node) {
    const intersection = intersectingText(node, region);
    measuredText ||= intersection.measured;
    if (intersection.text) textParts.push(intersection.text);
    node = walker.nextNode() as Text | null;
  }
  if (!measuredText && intersectingVisual.length === 0) {
    const fallback = normalizeText(block.textContent ?? '');
    if (fallback) textParts.push(fallback);
  }
  const text = normalizeText(textParts.join(' '));
  if ([...text].length > DOCX_REGION_CAPTURE_LIMITS.maxTextCodePoints)
    throw new Error('DOCX_REGION_CAPTURE_LIMIT_EXCEEDED');
  const informative = [...text].filter((char) => /[\p{L}\p{N}]/u.test(char));
  return {
    text: text || null,
    reliableText: informative.length >= 8 && intersectingVisual.length === 0,
    hasMixedContent: Boolean(text) && intersectingVisual.length > 0,
  };
}

function intersectingText(
  node: Text,
  region: PixelRect,
): { measured: boolean; text: string } {
  const probe = node.ownerDocument.createRange();
  probe.selectNodeContents(node);
  if (typeof probe.getClientRects !== 'function')
    return { measured: false, text: '' };
  const measured = probe.getClientRects().length > 0;
  probe.detach?.();
  if (!measured) return { measured: false, text: '' };
  const pieces: Array<{ text: string; character?: boolean }> = [];
  const tokens = textTokens(node.data);
  for (const token of tokens) {
    const range = node.ownerDocument.createRange();
    range.setStart(node, token.start);
    range.setEnd(node, token.end);
    if ([...range.getClientRects()].some((item) => intersects(item, region)))
      pieces.push(token);
    range.detach?.();
  }
  const separator = pieces.every((piece) => piece.character) ? '' : ' ';
  return {
    measured: true,
    text: normalizeText(pieces.map((piece) => piece.text).join(separator)),
  };
}

function textTokens(
  value: string,
): Array<{ start: number; end: number; text: string; character?: boolean }> {
  const words = [...value.matchAll(/\S+/gu)].map((match) => ({
    start: match.index,
    end: match.index + match[0].length,
    text: match[0],
  }));
  if (
    words.length !== 1 ||
    /\s/u.test(value) ||
    !/[\p{Script=Han}\p{Script=Hiragana}\p{Script=Katakana}\p{Script=Hangul}]/u.test(
      value,
    )
  )
    return words;
  const characters: Array<{
    start: number;
    end: number;
    text: string;
    character: true;
  }> = [];
  let offset = 0;
  for (const character of value) {
    characters.push({
      start: offset,
      end: offset + character.length,
      text: character,
      character: true,
    });
    offset += character.length;
  }
  return characters;
}

async function captureDomRegion(
  block: HTMLElement,
  rect: NormalizedRect,
  preferWholeDom: boolean,
  signal?: AbortSignal,
) {
  const bounds = block.getBoundingClientRect();
  const source = denormalize(bounds, rect);
  if (source.width <= 0 || source.height <= 0)
    throw new Error('DOCX_REGION_CAPTURE_UNAVAILABLE');
  const scale = boundedScale(source.width, source.height);
  const width = Math.max(1, Math.floor(source.width * scale));
  const height = Math.max(1, Math.floor(source.height * scale));
  const canvas = block.ownerDocument.createElement('canvas');
  canvas.width = width;
  canvas.height = height;
  try {
    const context = canvas.getContext('2d', { alpha: false });
    if (!context) throw new Error('DOCX_REGION_CAPTURE_FAILED');
    let rendered = false;
    if (preferWholeDom || block.matches('math,table')) {
      try {
        await drawDomSnapshot(context, block, source, width, height, signal);
        rendered = true;
      } catch (error) {
        if (isAbort(error)) throw error;
      }
    }
    if (!rendered)
      rendered = drawNativeVisuals(context, block, source, width, height);
    if (!rendered)
      await drawDomSnapshot(context, block, source, width, height, signal);
    const blob = await canvasBlob(canvas);
    throwIfAborted(signal);
    if (
      !blob ||
      blob.type !== 'image/png' ||
      blob.size > DOCX_REGION_CAPTURE_LIMITS.maxBytes
    )
      throw new Error('DOCX_REGION_CAPTURE_LIMIT_EXCEEDED');
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
    canvas.width = 0;
    canvas.height = 0;
  }
}

function drawNativeVisuals(
  context: CanvasRenderingContext2D,
  block: HTMLElement,
  region: PixelRect,
  width: number,
  height: number,
): boolean {
  const candidates = [
    ...(block.matches('img,canvas') ? [block] : []),
    ...block.querySelectorAll<HTMLElement>('img,canvas'),
  ];
  let rendered = false;
  for (const candidate of unique(candidates)) {
    const bounds = candidate.getBoundingClientRect();
    const overlap = intersection(bounds, region);
    if (!overlap) continue;
    try {
      const isCanvas = candidate.tagName.toLowerCase() === 'canvas';
      const sourceWidth = isCanvas
        ? (candidate as HTMLCanvasElement).width
        : (candidate as HTMLImageElement).naturalWidth;
      const sourceHeight = isCanvas
        ? (candidate as HTMLCanvasElement).height
        : (candidate as HTMLImageElement).naturalHeight;
      if (sourceWidth <= 0 || sourceHeight <= 0) continue;
      context.drawImage(
        candidate as CanvasImageSource,
        ((overlap.left - bounds.left) / bounds.width) * sourceWidth,
        ((overlap.top - bounds.top) / bounds.height) * sourceHeight,
        (overlap.width / bounds.width) * sourceWidth,
        (overlap.height / bounds.height) * sourceHeight,
        ((overlap.left - region.left) / region.width) * width,
        ((overlap.top - region.top) / region.height) * height,
        (overlap.width / region.width) * width,
        (overlap.height / region.height) * height,
      );
      rendered = true;
    } catch {
      // Refuse tainted sources and fall back to a sanitized local DOM snapshot.
    }
  }
  return rendered;
}

async function drawDomSnapshot(
  context: CanvasRenderingContext2D,
  block: HTMLElement,
  region: PixelRect,
  width: number,
  height: number,
  signal?: AbortSignal,
): Promise<void> {
  const clone = sanitizeClone(block);
  const blockBounds = block.getBoundingClientRect();
  const serialized = new XMLSerializer().serializeToString(clone);
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="${blockBounds.width}" height="${blockBounds.height}"><foreignObject width="100%" height="100%"><div xmlns="http://www.w3.org/1999/xhtml">${serialized}</div></foreignObject></svg>`;
  const encoded = new TextEncoder().encode(svg);
  if (encoded.byteLength > DOCX_REGION_CAPTURE_LIMITS.maxCanonicalBytes) {
    encoded.fill(0);
    throw new Error('DOCX_REGION_CAPTURE_LIMIT_EXCEEDED');
  }
  encoded.fill(0);
  const url = URL.createObjectURL(
    new Blob([svg], { type: 'image/svg+xml;charset=utf-8' }),
  );
  try {
    const image = await loadLocalImage(url, signal);
    throwIfAborted(signal);
    context.drawImage(
      image,
      region.left - blockBounds.left,
      region.top - blockBounds.top,
      region.width,
      region.height,
      0,
      0,
      width,
      height,
    );
  } finally {
    URL.revokeObjectURL(url);
  }
}

function sanitizeClone(block: HTMLElement): HTMLElement {
  const clone = block.cloneNode(true) as HTMLElement;
  clone
    .querySelectorAll('script,iframe,object,embed,form,link,style,svg')
    .forEach((node) => node.remove());
  for (const node of [clone, ...clone.querySelectorAll<HTMLElement>('*')]) {
    for (const attribute of [...node.attributes]) {
      const name = attribute.name.toLowerCase();
      if (
        name.startsWith('on') ||
        name.endsWith('href') ||
        /^(?:srcset|poster|background)$/u.test(name) ||
        name === 'style' ||
        (name === 'src' &&
          !/^data:image\/(?:png|jpeg|gif|webp);base64,[a-z0-9+/]+={0,2}$/iu.test(
            attribute.value,
          ))
      )
        node.removeAttribute(attribute.name);
    }
  }
  return clone;
}

function canonicalBlock(block: Element): string {
  const parts: string[] = [];
  const stack: Array<{ node: Node; closing: boolean }> = [
    { node: block, closing: false },
  ];
  let count = 0;
  while (stack.length > 0) {
    const frame = stack.pop()!;
    if (frame.closing) {
      parts.push(`</${(frame.node as Element).tagName.toLowerCase()}>`);
      continue;
    }
    count += 1;
    if (count > DOCX_REGION_CAPTURE_LIMITS.maxDomNodes)
      throw new Error('DOCX_REGION_CAPTURE_LIMIT_EXCEEDED');
    appendCanonical(frame.node, parts);
    if (frame.node.nodeType !== Node.ELEMENT_NODE) continue;
    stack.push({ node: frame.node, closing: true });
    const children = [...frame.node.childNodes];
    for (let index = children.length - 1; index >= 0; index -= 1)
      stack.push({ node: children[index]!, closing: false });
  }
  return parts.join('');
}

function appendCanonical(node: Node, parts: string[]): void {
  if (node.nodeType === Node.TEXT_NODE) {
    const text = normalizeText(node.textContent ?? '');
    if (text) parts.push(`#${text}`);
    return;
  }
  const element = node as Element;
  const attributes = [...element.attributes]
    .filter(
      (attribute) =>
        !/^(?:id|class|style|data-block-id|data-section-id)$/iu.test(
          attribute.name,
        ),
    )
    .filter((attribute) => !attribute.name.toLowerCase().startsWith('on'))
    .sort((a, b) => a.name.localeCompare(b.name))
    .map((attribute) => `${attribute.name}=${attribute.value.normalize('NFC')}`)
    .join(';');
  parts.push(`<${element.tagName.toLowerCase()}|${attributes}>`);
}

function quoteFor(block: Element, exact: string | null): TextQuote | null {
  if (!exact) return null;
  const whole = normalizeText(block.textContent ?? '');
  const index = whole.indexOf(exact);
  if (index < 0) return { exact, prefix: '', suffix: '' };
  return {
    exact,
    prefix: [...whole.slice(0, index)].slice(-64).join(''),
    suffix: [...whole.slice(index + exact.length)].slice(0, 64).join(''),
  };
}

function docxRegionAnchor(
  data: DocxRegionCaptureData,
  contentSha256: string,
  textFallback: TextQuote | null,
): ContentAnchor {
  return {
    kind: 'region',
    region: {
      locator: { format: 'docx', blockId: data.blockId },
      rect: { ...data.rect },
      contentSha256,
      textFallback,
    },
  };
}

type PixelRect = {
  left: number;
  top: number;
  right: number;
  bottom: number;
  width: number;
  height: number;
};
function denormalize(
  bounds: Pick<DOMRect, 'left' | 'top' | 'width' | 'height'>,
  rect: NormalizedRect,
): PixelRect {
  const left = bounds.left + rect.x * bounds.width;
  const top = bounds.top + rect.y * bounds.height;
  const width = rect.width * bounds.width;
  const height = rect.height * bounds.height;
  return {
    left,
    top,
    right: left + width,
    bottom: top + height,
    width,
    height,
  };
}
function intersects(
  bounds: Pick<DOMRect, 'left' | 'top' | 'right' | 'bottom'>,
  region: PixelRect,
): boolean {
  return (
    bounds.right > region.left &&
    bounds.left < region.right &&
    bounds.bottom > region.top &&
    bounds.top < region.bottom
  );
}
function intersection(
  bounds: Pick<
    DOMRect,
    'left' | 'top' | 'right' | 'bottom' | 'width' | 'height'
  >,
  region: PixelRect,
): PixelRect | null {
  if (bounds.width <= 0 || bounds.height <= 0 || !intersects(bounds, region))
    return null;
  const left = Math.max(bounds.left, region.left);
  const top = Math.max(bounds.top, region.top);
  const right = Math.min(bounds.right, region.right);
  const bottom = Math.min(bounds.bottom, region.bottom);
  return {
    left,
    top,
    right,
    bottom,
    width: right - left,
    height: bottom - top,
  };
}
function boundedScale(width: number, height: number): number {
  const deviceScale = Math.min(
    DOCX_REGION_CAPTURE_LIMITS.maxScale,
    Math.max(1, globalThis.devicePixelRatio || 1),
  );
  return Math.min(
    deviceScale,
    DOCX_REGION_CAPTURE_LIMITS.maxWidth / width,
    DOCX_REGION_CAPTURE_LIMITS.maxHeight / height,
    Math.sqrt(DOCX_REGION_CAPTURE_LIMITS.maxPixels / (width * height)),
  );
}
function canvasBlob(canvas: HTMLCanvasElement): Promise<Blob | null> {
  return new Promise((resolve) => canvas.toBlob(resolve, 'image/png'));
}
function loadLocalImage(
  url: string,
  signal?: AbortSignal,
): Promise<HTMLImageElement> {
  return new Promise((resolve, reject) => {
    const image = new Image();
    const abort = () => {
      image.src = '';
      reject(new DOMException('Aborted', 'AbortError'));
    };
    image.onload = () => {
      signal?.removeEventListener('abort', abort);
      resolve(image);
    };
    image.onerror = () => {
      signal?.removeEventListener('abort', abort);
      reject(new Error('DOCX_REGION_CAPTURE_FAILED'));
    };
    signal?.addEventListener('abort', abort, { once: true });
    if (signal?.aborted) abort();
    else image.src = url;
  });
}
function validateRect(rect: NormalizedRect): void {
  if (!isValidRect(rect)) throw new Error('DOCX_REGION_CAPTURE_UNAVAILABLE');
}
function isValidRect(rect: NormalizedRect): boolean {
  return (
    [rect.x, rect.y, rect.width, rect.height].every(Number.isFinite) &&
    rect.x >= 0 &&
    rect.y >= 0 &&
    rect.width > 0 &&
    rect.height > 0 &&
    rect.x + rect.width <= 1.000001 &&
    rect.y + rect.height <= 1.000001
  );
}
function normalizeText(value: string): string {
  return value.normalize('NFC').replace(/\s+/gu, ' ').trim();
}
function codePointLengthExceeds(value: string, limit: number): boolean {
  let count = 0;
  for (let offset = 0; offset < value.length;) {
    const codePoint = value.codePointAt(offset)!;
    offset += codePoint > 0xffff ? 2 : 1;
    count += 1;
    if (count > limit) return true;
  }
  return false;
}
function unique<T>(items: readonly T[]): T[] {
  return [...new Set(items)];
}
async function sha256(bytes: Uint8Array): Promise<string> {
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
function constantTimeEqual(left: string, right: string): boolean {
  if (left.length !== right.length) return false;
  let difference = 0;
  for (let index = 0; index < left.length; index += 1)
    difference |= left.charCodeAt(index) ^ right.charCodeAt(index);
  return difference === 0;
}
function throwIfAborted(signal?: AbortSignal): void {
  if (signal?.aborted) throw new DOMException('Aborted', 'AbortError');
}
function isAbort(error: unknown): boolean {
  return error instanceof DOMException && error.name === 'AbortError';
}
const VISUAL_SELECTOR = 'img,canvas,math,figure,table,picture';
const LOWER_SHA256 = /^[0-9a-f]{64}$/;
