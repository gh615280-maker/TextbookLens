import type { BlockKind } from '../../../lib/generated/document';
import { normalizeWhitespace } from './pdf-layout';

export interface HtmlBlock {
  element: Element;
  kind: BlockKind;
  text: string;
}

const selector = 'h1, h2, h3, h4, h5, h6, p, li, table, figcaption, math';

/** Extracts only displayable EPUB structure, never script/style fallback content. */
export function collectHtmlBlocks(document: Document): HtmlBlock[] {
  return [...document.querySelectorAll(selector)]
    .filter((element) => !isAbsorbedByContainer(element))
    .map((element) => ({
      element,
      kind: blockKind(element),
      text: blockText(element),
    }))
    .filter((block) => block.text.length > 0);
}

function isAbsorbedByContainer(element: Element): boolean {
  if (element.localName === 'table') return false;
  if (element.parentElement?.closest('table')) return true;
  return (
    element.localName !== 'li' && element.parentElement?.closest('li') !== null
  );
}

function blockKind(element: Element): BlockKind {
  if (/^h[1-6]$/iu.test(element.localName)) return 'heading';
  if (element.localName === 'li') return 'list';
  if (element.localName === 'table') return 'table';
  if (element.localName === 'figcaption') return 'caption';
  if (element.localName === 'math') return 'equation';
  return 'paragraph';
}

export function isEquationText(value: string): boolean {
  return (
    /[\p{L}\p{N}⁰¹²³⁴⁵⁶⁷⁸⁹]\s*(?:=|[+*/×÷−])\s*[\p{L}\p{N}⁰¹²³⁴⁵⁶⁷⁸⁹]/u.test(
      value,
    ) ||
    /(?:[\p{L}\p{N}]\s*-\s*\p{N}|\p{N}\s*-\s*[\p{L}\p{N}])/u.test(value) ||
    /[\p{L}\p{N}][⁰¹²³⁴⁵⁶⁷⁸⁹]/u.test(value)
  );
}

function blockText(element: Element): string {
  if (element.localName !== 'table')
    return normalizeWhitespace(element.textContent ?? '');
  return [...element.querySelectorAll('tr')]
    .map((row) =>
      [...row.querySelectorAll('th, td')]
        .map((cell) => normalizeWhitespace(cell.textContent ?? ''))
        .filter(Boolean)
        .join('\t'),
    )
    .filter(Boolean)
    .join('\n');
}
