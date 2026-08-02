import ePub, { type Book } from 'epubjs';

import type { NormalizedBlockInput, NormalizedSectionInput } from '../../../lib/generated/document';
import type { UserFacingError } from '../../../lib/errors';
import { stableBlockId, stableSectionId } from '../id';
import type { DocumentParser, ParseContext, ParserSink } from '../parser-contract';
import { collectHtmlBlocks } from './html-blocks';

type EpubSection = {
  index: number;
  href: string;
  load(request?: unknown): Promise<Document>;
  unload(): void;
  cfiFromElement(element: Element): string;
};

class EpubParserError extends Error implements UserFacingError {
  readonly nextStep = 'Choose a valid, unencrypted EPUB with readable text.';
  readonly diagnosticId = null;

  constructor(
    readonly code: 'FILE_CORRUPTED' | 'FILE_ENCRYPTED_OR_DRM' | 'NO_EXTRACTABLE_TEXT',
    cause?: unknown,
  ) {
    super(code, { cause });
    this.name = 'EpubParserError';
  }
}

export class EpubParser implements DocumentParser {
  readonly format = 'epub' as const;

  async parse(context: ParseContext, sink: ParserSink): Promise<void> {
    throwIfAborted(context.signal);
    let book: Book | undefined;
    try {
      book = ePub({ replacements: 'none' });
      await book.open(context.source);
      await book.ready;
      throwIfAborted(context.signal);
      const metadata = book.packaging.metadata;
      await sink.begin({
        title: metadata.title.trim() || '未命名 EPUB',
        author: metadata.creator.trim() || null,
        language: metadata.language.trim() || null,
      });

      const spine: EpubSection[] = [];
      book.spine.each((section: EpubSection) => spine.push(section));
      let meaningfulCharacters = 0;
      for (const [ordinal, section] of spine.entries()) {
        throwIfAborted(context.signal);
        try {
          const document = await section.load(book.load.bind(book));
          const htmlBlocks = collectHtmlBlocks(document);
          meaningfulCharacters += htmlBlocks.reduce(
            (total, block) => total + countMeaningfulCharacters(block.text),
            0,
          );
          if (htmlBlocks.length > 0) {
            const sectionId = stableSectionId(context.bookId, ordinal);
            const firstCfi = section.cfiFromElement(htmlBlocks[0]!.element);
            const blocks = htmlBlocks.map((block, blockOrdinal): NormalizedBlockInput => ({
              id: stableBlockId(context.bookId, ordinal, blockOrdinal),
              ordinal: blockOrdinal,
              kind: block.kind,
              plainText: block.text,
              locator: { format: 'epub', cfi: section.cfiFromElement(block.element), sectionId },
            }));
            const heading = htmlBlocks.find((block) => block.kind === 'heading');
            const normalized: NormalizedSectionInput = {
              id: sectionId,
              parentId: null,
              ordinal,
              title: heading?.text ?? `第 ${ordinal + 1} 节`,
              locator: { format: 'epub', cfi: firstCfi, sectionId },
              blocks,
            };
            await sink.append([normalized]);
          }
        } finally {
          section.unload();
        }
        sink.progress({ stage: 'parsing', completed: ordinal + 1, total: spine.length, messageKey: 'import.parsing.epub' });
      }
      if (meaningfulCharacters === 0) throw new EpubParserError('NO_EXTRACTABLE_TEXT');
    } catch (error) {
      if (context.signal.aborted) throw abortError();
      throw classifyEpubError(error);
    } finally {
      book?.destroy();
    }
  }
}

function countMeaningfulCharacters(value: string): number {
  return [...value].filter((character) => /[\p{L}\p{N}]/u.test(character)).length;
}

function classifyEpubError(error: unknown): Error {
  if (error instanceof EpubParserError) return error;
  const message = error instanceof Error ? error.message : '';
  if (/drm|encrypted|encryption|password/iu.test(message)) return new EpubParserError('FILE_ENCRYPTED_OR_DRM', error);
  return new EpubParserError('FILE_CORRUPTED', error);
}

function throwIfAborted(signal: AbortSignal): void {
  if (signal.aborted) throw abortError();
}

function abortError(): DOMException {
  return new DOMException('The EPUB import was cancelled.', 'AbortError');
}
