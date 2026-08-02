import DOMPurify from 'dompurify';

import type {
  NormalizedBlockInput,
  NormalizedSectionInput,
} from '../../../lib/generated/document';
import { stableBlockId, stableSectionId } from '../id';
import { collectHtmlBlocks, isEquationText } from './html-blocks';

const allowedTags = [
  'h1',
  'h2',
  'h3',
  'h4',
  'h5',
  'h6',
  'p',
  'strong',
  'em',
  'b',
  'i',
  'u',
  'br',
  'ul',
  'ol',
  'li',
  'table',
  'thead',
  'tbody',
  'tfoot',
  'tr',
  'th',
  'td',
  'figure',
  'figcaption',
  'img',
  'math',
  'mrow',
  'mi',
  'mn',
  'mo',
  'msup',
  'msub',
  'mfrac',
  'annotation',
];
const allowedAttributes = ['alt', 'colspan', 'rowspan', 'src'];
const safeImageDataUrl =
  /^data:image\/(?:png|jpeg|gif|webp);base64,[a-z0-9+/]+={0,2}$/iu;

export interface SanitizedDocx {
  html: string;
  sections: NormalizedSectionInput[];
}

/** Sanitizes untrusted Mammoth HTML and binds persisted blocks to stable IDs. */
export function sanitizeDocxHtml(input: string, bookId: string): SanitizedDocx {
  const sanitized = DOMPurify.sanitize(input, {
    ALLOWED_TAGS: allowedTags,
    ALLOWED_ATTR: allowedAttributes,
    ALLOW_DATA_ATTR: false,
    ALLOW_ARIA_ATTR: false,
    FORBID_TAGS: [
      'script',
      'style',
      'svg',
      'iframe',
      'object',
      'embed',
      'link',
      'meta',
    ],
    FORBID_ATTR: ['style'],
    ALLOWED_URI_REGEXP: safeImageDataUrl,
    SAFE_FOR_XML: true,
    SANITIZE_NAMED_PROPS: true,
  });
  const document = new DOMParser().parseFromString(sanitized, 'text/html');
  for (const image of document.querySelectorAll('img')) {
    if (!safeImageDataUrl.test(image.getAttribute('src') ?? ''))
      image.removeAttribute('src');
  }

  const sections: NormalizedSectionInput[] = [];
  let sectionOrdinal = -1;
  let current:
    { id: string; title: string; blocks: NormalizedBlockInput[] } | undefined;
  for (const htmlBlock of collectHtmlBlocks(document)) {
    const kind = classifyDocxKind(htmlBlock.kind, htmlBlock.text);
    if (kind === 'heading' || !current) {
      sectionOrdinal += 1;
      current = {
        id: stableSectionId(bookId, sectionOrdinal),
        title:
          kind === 'heading' ? htmlBlock.text : `第 ${sectionOrdinal + 1} 节`,
        blocks: [],
      };
      sections.push({
        id: current.id,
        parentId: null,
        ordinal: sectionOrdinal,
        title: current.title,
        locator: {
          format: 'docx',
          startBlockId: '',
          startOffset: 0,
          endBlockId: '',
          endOffset: 0,
        },
        blocks: current.blocks,
      });
    }
    const blockOrdinal = current.blocks.length;
    const blockId = stableBlockId(bookId, sectionOrdinal, blockOrdinal);
    const endOffset = [...htmlBlock.text].length;
    htmlBlock.element.setAttribute('data-section-id', current.id);
    htmlBlock.element.setAttribute('data-block-id', blockId);
    current.blocks.push({
      id: blockId,
      ordinal: blockOrdinal,
      kind,
      plainText: htmlBlock.text,
      locator: {
        format: 'docx',
        startBlockId: blockId,
        startOffset: 0,
        endBlockId: blockId,
        endOffset,
      },
    });
  }

  for (const section of sections) {
    const first = section.blocks[0];
    const last = section.blocks.at(-1);
    if (first && last) {
      section.locator = {
        format: 'docx',
        startBlockId: first.id,
        startOffset: 0,
        endBlockId: last.id,
        endOffset: [...last.plainText].length,
      };
    }
  }
  return {
    html: `<!doctype html><html><body>${document.body.innerHTML}</body></html>`,
    sections,
  };
}

function classifyDocxKind(
  kind: NormalizedBlockInput['kind'],
  text: string,
): NormalizedBlockInput['kind'] {
  if (kind !== 'paragraph') return kind;
  if (/^(?:图|figure)\s*\d/iu.test(text)) return 'caption';
  if (isEquationText(text)) return 'equation';
  return kind;
}
