import {
  getDocument,
  GlobalWorkerOptions,
} from 'pdfjs-dist/legacy/build/pdf.mjs';
import pdfWorkerSrc from 'pdfjs-dist/legacy/build/pdf.worker.mjs?url';
import { localPdfOptions } from '../../../lib/pdf-options';
import type { TextItem } from 'pdfjs-dist/types/src/display/api';

import type {
  NormalizedBlockInput,
  NormalizedSectionInput,
} from '../../../lib/generated/document';
import type { UserFacingError } from '../../../lib/errors';
import { assessPdfPageQuality } from '../../indexing/pdf-quality';
import type { LocalPdfPageQualityDto } from '../../indexing/indexing-contract';
import { stableBlockId, stableSectionId } from '../id';
import type {
  DocumentParser,
  ParseContext,
  ParserSink,
} from '../parser-contract';
import { isEquationText } from './html-blocks';
import {
  normalizePdfPage,
  normalizeWhitespace,
  pdfTextItems,
} from './pdf-layout';

// Keep the worker on the same legacy build as the display API and let Vite
// emit its URL. A package-specifier URL is not a browser-resolvable worker URL
// in the packaged Tauri webview.
GlobalWorkerOptions.workerSrc = pdfWorkerSrc;

class PdfParserError extends Error implements UserFacingError {
  readonly nextStep = 'Choose a readable, unencrypted PDF.';
  readonly diagnosticId = null;

  constructor(
    readonly code: 'FILE_CORRUPTED' | 'FILE_ENCRYPTED_OR_DRM',
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
    let document:
      Awaited<ReturnType<typeof getDocument>['promise']> | undefined;
    try {
      loadingTask = getDocument(localPdfOptions(context.source));
      document = await loadingTask.promise;
      const metadata = await document.getMetadata().catch(() => null);
      const info = metadata?.info as
        { Title?: string; Author?: string } | undefined;
      const sections: NormalizedSectionInput[] = [];

      for (
        let pageNumber = 1;
        pageNumber <= document.numPages;
        pageNumber += 1
      ) {
        throwIfAborted(context.signal);
        const page = await document.getPage(pageNumber);
        let blocks;
        try {
          const textContent = await page.getTextContent();
          throwIfAborted(context.signal);
          blocks = normalizePdfPage(
            pdfTextItems(textContent.items.filter(isTextItem)),
          );
        } finally {
          page.cleanup();
        }
        const sectionOrdinal = pageNumber - 1;
        const sectionId = stableSectionId(context.bookId, sectionOrdinal);
        sections.push({
          id: sectionId,
          parentId: null,
          ordinal: sectionOrdinal,
          title: `第 ${pageNumber} 页`,
          locator: {
            format: 'pdf',
            startPage: pageNumber,
            endPage: pageNumber,
            rectsByPage: null,
          },
          blocks: blocks.map((block, blockOrdinal): NormalizedBlockInput => ({
            id: stableBlockId(context.bookId, sectionOrdinal, blockOrdinal),
            ordinal: blockOrdinal,
            kind: isEquationText(block.text) ? 'equation' : 'paragraph',
            plainText: block.text,
            locator: {
              format: 'pdf',
              startPage: pageNumber,
              endPage: pageNumber,
              rectsByPage: null,
            },
          })),
        });
        sink.progress({
          stage: 'parsing',
          completed: pageNumber,
          total: document.numPages,
          messageKey: 'import.parsing.pdf',
        });
      }

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

/**
 * Reads a local in-memory PDF with PDF.js and returns only safe quality candidates.
 * The import parser does not persist or upload this information; Task 4 owns run creation.
 */
export async function inspectLocalPdfPageQuality(
  source: ArrayBuffer,
  signal: AbortSignal,
): Promise<LocalPdfPageQualityDto[]> {
  throwIfAborted(signal);
  const loadingTask = getDocument(localPdfOptions(source) as never);
  let document: Awaited<ReturnType<typeof getDocument>['promise']> | undefined;
  try {
    document = await loadingTask.promise;
    const qualities: LocalPdfPageQualityDto[] = [];
    for (let pageNumber = 1; pageNumber <= document.numPages; pageNumber += 1) {
      throwIfAborted(signal);
      const page = await document.getPage(pageNumber);
      try {
        const [viewport, textContent] = await Promise.all([
          page.getViewport({ scale: 1, rotation: 0 }),
          page.getTextContent(),
        ]);
        throwIfAborted(signal);
        qualities.push(
          assessPdfPageQuality({
            pageNumber,
            width: viewport.width,
            height: viewport.height,
            items: pdfTextItems(textContent.items.filter(isTextItem)),
          }),
        );
      } finally {
        page.cleanup();
      }
    }
    return qualities;
  } catch (error) {
    if (signal.aborted) throw abortError();
    throw classifyPdfError(error);
  } finally {
    await document?.cleanup();
    await loadingTask.destroy();
  }
}

function isTextItem(item: TextItem | { type: string }): item is TextItem {
  return 'str' in item;
}

function classifyPdfError(error: unknown): Error {
  if (error instanceof PdfParserError) return error;
  const message = error instanceof Error ? error.message : '';
  if (/password|encrypted|encryption/iu.test(message))
    return new PdfParserError('FILE_ENCRYPTED_OR_DRM', error);
  return new PdfParserError('FILE_CORRUPTED', error);
}

function throwIfAborted(signal: AbortSignal): void {
  if (signal.aborted) throw abortError();
}

function abortError(): DOMException {
  return new DOMException('The PDF import was cancelled.', 'AbortError');
}
