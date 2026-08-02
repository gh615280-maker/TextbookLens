import type { BookFormat } from '../../lib/generated/book';
import type { UserFacingError } from '../../lib/errors';
import type { DocumentParser } from './parser-contract';
import { DocxParser } from './parsers/docx-parser';
import { EpubParser } from './parsers/epub-parser';
import { PdfParser } from './parsers/pdf-parser';

class ParserRegistryError extends Error implements UserFacingError {
  readonly nextStep: string;
  readonly diagnosticId = null;

  constructor(
    readonly code: UserFacingError['code'],
    message: string,
    nextStep: string,
  ) {
    super(message);
    this.name = 'ParserRegistryError';
    this.nextStep = nextStep;
  }
}

/** The application composition root registers every supported local adapter. */
export function createDocumentParserRegistry(): ParserRegistry {
  const registry = new ParserRegistry([
    new PdfParser(),
    new EpubParser(),
    new DocxParser(),
  ]);
  registry.assertComplete();
  return registry;
}

export class ParserRegistry {
  readonly #parsers = new Map<BookFormat, DocumentParser>();

  constructor(parsers: readonly DocumentParser[] = []) {
    for (const parser of parsers) this.register(parser);
  }

  register(parser: DocumentParser): void {
    if (this.#parsers.has(parser.format)) {
      throw new ParserRegistryError(
        'REQUEST_CONFLICT',
        `Parser already registered for ${parser.format}.`,
        'Register each document parser exactly once.',
      );
    }
    this.#parsers.set(parser.format, parser);
  }

  get(format: string): DocumentParser {
    const parser = this.#parsers.get(format as BookFormat);
    if (!parser) {
      throw new ParserRegistryError(
        'UNSUPPORTED_FILE_TYPE',
        '不支持该文件类型。',
        '请选择 PDF、EPUB 或 DOCX 文件。',
      );
    }
    return parser;
  }

  /** Called by the Tasks 4–6 composition root after all real adapters exist. */
  assertComplete(): void {
    for (const format of ['pdf', 'epub', 'docx'] as const) {
      this.get(format);
    }
  }
}
