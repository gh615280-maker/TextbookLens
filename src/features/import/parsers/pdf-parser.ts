import { getDocument, GlobalWorkerOptions } from 'pdfjs-dist/legacy/build/pdf.mjs';
import type { TextItem } from 'pdfjs-dist/types/src/display/api';

import type { NormalizedBlockInput, NormalizedSectionInput } from '../../../lib/generated/document';
import { stableBlockId, stableSectionId } from '../id';
import type { DocumentParser, ParseContext, ParserSink } from '../parser-contract';
import { normalizePdfPage, normalizeWhitespace } from './pdf-layout';

GlobalWorkerOptions.workerSrc = new URL(
  'pdfjs-dist/build/pdf.worker.mjs',
  import.meta.url,
).toString();

class PdfParserError extends Error {
  constructor(
    readonly code: 'FILE_CORRUPTED' | 'FILE_ENCRYPTED_OR_DRM' | 'NO_EXTRACTABLE_TEXT',
    cause?: unknown,
  ) {
    super(code, { cause });
    this.name = 'PdfParserError';
  }
}

export class PdfParser implements DocumentParser {
  readonly format = 'pdf' as const;

  async parse(context: ParseContext, sink: ParserSink): Promise<void> {
    throwIfAborted(context.signal);
    let loadingTask: ReturnType<typeof getDocument> | undefined;
    let document: Awaited<ReturnType<typeof getDocument>['promise']> | undefined;
    try {
      loadingTask = getDocument({ data: new Uint8Array(context.source) });
      document = await loadingTask.promise;
      const metadata = await document.getMetadata().catch(() => null);
      const info = metadata?.info as { Title?: string; Author?: string } | undefined;
      const sections: NormalizedSectionInput[] = [];
      let meaningfulCharacters = 0;

      for (let pageNumber = 1; pageNumber <= document.numPages; pageNumber += 1) {
        throwIfAborted(context.signal);
        const page = await document.getPage(pageNumber);
        const textContent = await page.getTextContent();
        throwIfAborted(context.signal);
        const blocks = normalizePdfPage(
          textContent.items.filter(isTextItem).map(({ str, transform, width }) => ({ str, transform, width })),
        );
        meaningfulCharacters += blocks.reduce(
          (total, block) => total + countMeaningfulCharacters(block.text),
          0,
        );
        const sectionOrdinal = pageNumber - 1;
        const sectionId = stableSectionId(context.bookId, sectionOrdinal);
        sections.push({
          id: sectionId,
          parentId: null,
          ordinal: sectionOrdinal,
          title: `第 ${pageNumber} 页`,
          locator: { format: 'pdf', startPage: pageNumber, endPage: pageNumber, rectsByPage: null },
          blocks: blocks.map((block, blockOrdinal): NormalizedBlockInput => ({
            id: stableBlockId(context.bookId, sectionOrdinal, blockOrdinal),
            ordinal: blockOrdinal,
            kind: isEquation(block.text) ? 'code' : 'paragraph',
            plainText: block.text,
            locator: { format: 'pdf', startPage: pageNumber, endPage: pageNumber, rectsByPage: null },
          })),
        });
        sink.progress({ stage: 'parsing', completed: pageNumber, total: document.numPages, messageKey: 'import.parsing.pdf' });
      }

      if (meaningfulCharacters === 0) throw new PdfParserError('NO_EXTRACTABLE_TEXT');
      await sink.begin({
        title: normalizeWhitespace(info?.Title ?? '') || '未命名 PDF',
        author: normalizeWhitespace(info?.Author ?? '') || null,
        language: null,
      });
      await sink.append(sections);
    } catch (error) {
      if (context.signal.aborted) throw abortError();
      throw classifyPdfError(error);
    } finally {
      await document?.cleanup();
      await loadingTask?.destroy();
    }
  }
}

function isTextItem(item: TextItem | { type: string }): item is TextItem {
  return 'str' in item;
}

function countMeaningfulCharacters(value: string): number {
  return [...value].filter((character) => /[\p{L}\p{N}]/u.test(character)).length;
}

function isEquation(value: string): boolean {
  return /(?:=|[+\-*/×÷]|[⁰¹²³⁴⁵⁶⁷⁸⁹])/u.test(value) && /[\p{L}\p{N}]/u.test(value);
}

function classifyPdfError(error: unknown): Error {
  if (error instanceof PdfParserError) return error;
  const message = error instanceof Error ? error.message : '';
  if (/password|encrypted|encryption/iu.test(message)) return new PdfParserError('FILE_ENCRYPTED_OR_DRM', error);
  return new PdfParserError('FILE_CORRUPTED', error);
}

function throwIfAborted(signal: AbortSignal): void {
  if (signal.aborted) throw abortError();
}

function abortError(): DOMException {
  return new DOMException('The PDF import was cancelled.', 'AbortError');
}
