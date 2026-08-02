import type { TextQuote } from '../../../lib/generated/document';
import { createTextQuote } from '../anchors/text-quote';

export function snapshotEpubRange(
  range: Range,
  sectionId: string,
  cfi: string,
): { text: string; sectionId: string; cfi: string; quote: TextQuote } | null {
  const text = range.toString();
  if (!text || !cfi) return null;
  const root =
    range.commonAncestorContainer.nodeType === Node.ELEMENT_NODE
      ? (range.commonAncestorContainer as Element)
      : range.commonAncestorContainer.parentElement;
  const sectionText = root?.ownerDocument?.body.textContent ?? text;
  const start = sectionText.indexOf(text);
  return {
    text,
    sectionId,
    cfi,
    quote: createTextQuote(
      sectionText,
      start < 0 ? 0 : [...sectionText.slice(0, start)].length,
      start < 0
        ? [...text].length
        : [...sectionText.slice(0, start + text.length)].length,
    ),
  };
}

/** Removes executable/external content before EPUB.js displays an archive document. */
export function sanitizeEpubDocument(document: Document): void {
  const frame = document.defaultView?.frameElement;
  if (frame instanceof HTMLIFrameElement)
    frame.setAttribute('sandbox', 'allow-same-origin');
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
  return !/^(?:https?:|javascript:|file:|data:text\/html)/iu.test(value);
}
