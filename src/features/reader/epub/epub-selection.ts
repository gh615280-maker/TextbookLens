import type { TextQuote } from '../../../lib/generated/document';
import { createTextQuote } from '../anchors/text-quote';

export function snapshotEpubRange(
  range: Range,
  sectionId: string,
  cfi: string,
): { text: string; sectionId: string; cfi: string; quote: TextQuote } | null {
  const text = range.toString();
  if (!text || !cfi) return null;
  const body = range.startContainer.ownerDocument?.body;
  if (
    !body ||
    !body.contains(range.startContainer) ||
    !body.contains(range.endContainer)
  )
    return null;
  const sectionText = body.textContent ?? '';
  const prefix = range.cloneRange();
  prefix.selectNodeContents(body);
  prefix.setEnd(range.startContainer, range.startOffset);
  const start = [...prefix.toString()].length;
  return {
    text,
    sectionId,
    cfi,
    quote: createTextQuote(sectionText, start, start + [...text].length),
  };
}

/** Translate the rendered iframe selection into the application's viewport. */
export function epubSelectionPosition(
  range: Range,
): { x: number; y: number } | undefined {
  if (typeof range.getBoundingClientRect !== 'function') return undefined;
  const rect = range.getBoundingClientRect();
  if (rect.width === 0 && rect.height === 0) return undefined;
  const frame = range.startContainer.ownerDocument?.defaultView?.frameElement;
  if (!(frame instanceof HTMLElement)) return { x: rect.left, y: rect.bottom };
  const bounds = frame.getBoundingClientRect();
  const sx = frame.offsetWidth ? bounds.width / frame.offsetWidth : 1;
  const sy = frame.offsetHeight ? bounds.height / frame.offsetHeight : 1;
  return {
    x: bounds.left + (frame.clientLeft + rect.left) * sx,
    y: bounds.top + (frame.clientTop + rect.bottom) * sy,
  };
}

/** Removes executable/external content before EPUB.js displays an archive document. */
export function sanitizeEpubDocument(document: Document): void {
  const frame = document.defaultView?.frameElement;
  if (frame instanceof HTMLIFrameElement)
    frame.setAttribute('sandbox', 'allow-same-origin');
  // This also confines CSS imports and image/font URLs that are not ordinary
  // href/src attributes. It is inserted before the archived document renders.
  const head = document.head ?? document.querySelector('head');
  document.querySelectorAll('meta[http-equiv]').forEach((meta) => {
    if (
      ['refresh', 'content-security-policy'].includes(
        meta.getAttribute('http-equiv')!.toLowerCase(),
      )
    )
      meta.remove();
  });
  if (head) {
    const policy = document.createElementNS(head.namespaceURI, 'meta');
    policy.setAttribute('data-textbooklens-csp', '');
    policy.setAttribute('http-equiv', 'Content-Security-Policy');
    policy.setAttribute(
      'content',
      "default-src 'none'; img-src blob: data:; style-src 'unsafe-inline' blob: data:; font-src blob: data:; script-src 'none'; connect-src 'none'; frame-src 'none'; form-action 'none'",
    );
    head.prepend(policy);
  }
  document
    .querySelectorAll('script,iframe,object,embed,form')
    .forEach((node) => node.remove());
  document.querySelectorAll<HTMLElement>('*').forEach((element) => {
    for (const attribute of [...element.attributes]) {
      if (attribute.name.toLowerCase().startsWith('on'))
        element.removeAttribute(attribute.name);
    }
    for (const name of ['href', 'src']) {
      const value = element.getAttribute(name);
      if (value && !isArchiveResource(value)) element.removeAttribute(name);
    }
  });
}

function isArchiveResource(value: string): boolean {
  const url = value.trim();
  if (/^(?:blob:|data:image\/)/iu.test(url)) return true;
  return !/^(?:[a-z][a-z\d+.-]*:|\/\/)/iu.test(url);
}
