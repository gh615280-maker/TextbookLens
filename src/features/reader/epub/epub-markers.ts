import type { TextQuote } from '../../../lib/generated/document';
import { findTextQuote } from '../anchors/text-quote';

export function recoverEpubCfi(
  section: { document: Document; cfiFromElement(element: Element): string },
  quote: TextQuote,
): string | null {
  const match = findTextQuote(section.document.body.textContent ?? '', quote);
  if (!match) return null;
  const walker = section.document.createTreeWalker(
    section.document.body,
    NodeFilter.SHOW_TEXT,
  );
  let seen = 0;
  let node = walker.nextNode();
  while (node) {
    const length = node.textContent?.length ?? 0;
    if (match.startUtf16 >= seen && match.startUtf16 <= seen + length)
      return section.cfiFromElement(
        node.parentElement ?? section.document.body,
      );
    seen += length;
    node = walker.nextNode();
  }
  return null;
}
