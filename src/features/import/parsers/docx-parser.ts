import mammoth from 'mammoth';

import type { UserFacingError } from '../../../lib/errors';
import type {
  DocumentParser,
  ParseContext,
  ParserSink,
} from '../parser-contract';
import { sanitizeDocxHtml } from './docx-html';

class DocxParserError extends Error implements UserFacingError {
  readonly nextStep = 'Choose a valid, unencrypted DOCX with readable text.';
  readonly diagnosticId = null;

  constructor(
    readonly code:
      'FILE_CORRUPTED' | 'FILE_ENCRYPTED_OR_DRM' | 'NO_EXTRACTABLE_TEXT',
    cause?: unknown,
  ) {
    super(code, { cause });
    this.name = 'DocxParserError';
  }
}

const styleMap = [
  "p[style-name='Heading 1'] => h1:fresh",
  "p[style-name='Heading 2'] => h2:fresh",
  "p[style-name='Caption'] => figcaption:fresh",
];

export class DocxParser implements DocumentParser {
  readonly format = 'docx' as const;

  async parse(context: ParseContext, sink: ParserSink): Promise<void> {
    throwIfAborted(context.signal);
    try {
      const converted = await mammoth.convertToHtml(
        mammothInput(context.source),
        { styleMap, externalFileAccess: false },
      );
      throwIfAborted(context.signal);
      const normalized = sanitizeDocxHtml(converted.value, context.bookId);
      if (countMeaningfulCharacters(normalized.sections) === 0) {
        throw new DocxParserError('NO_EXTRACTABLE_TEXT');
      }
      const title =
        normalized.sections.find((section) =>
          section.blocks.some((block) => block.kind === 'heading'),
        )?.title ?? '未命名 DOCX';
      await sink.begin({ title, author: null, language: null });
      await sink.writeDerivedText('document.html', normalized.html);
      await sink.append(normalized.sections);
      sink.progress({
        stage: 'parsing',
        completed: 1,
        total: 1,
        messageKey: 'import.parsing.docx',
      });
    } catch (error) {
      if (context.signal.aborted) throw abortError();
      throw classifyDocxError(error);
    }
  }
}

function mammothInput(arrayBuffer: ArrayBuffer): {
  arrayBuffer: ArrayBuffer;
  buffer?: Uint8Array;
} {
  const buffer = (
    globalThis as { Buffer?: { from(input: ArrayBuffer): Uint8Array } }
  ).Buffer;
  return buffer
    ? { arrayBuffer, buffer: buffer.from(arrayBuffer) }
    : { arrayBuffer };
}

function countMeaningfulCharacters(
  sections: ReturnType<typeof sanitizeDocxHtml>['sections'],
): number {
  return sections
    .flatMap((section) => section.blocks)
    .reduce(
      (total, block) =>
        total +
        [...block.plainText].filter((character) =>
          /[\p{L}\p{N}]/u.test(character),
        ).length,
      0,
    );
}

function classifyDocxError(error: unknown): Error {
  if (error instanceof DocxParserError) return error;
  const message = error instanceof Error ? error.message : '';
  if (/encrypted|encryption|password|drm/iu.test(message))
    return new DocxParserError('FILE_ENCRYPTED_OR_DRM', error);
  return new DocxParserError('FILE_CORRUPTED', error);
}

function throwIfAborted(signal: AbortSignal): void {
  if (signal.aborted) throw abortError();
}

function abortError(): DOMException {
  return new DOMException('The DOCX import was cancelled.', 'AbortError');
}
