import type { BookFormat, BookSummary } from '../../lib/generated/book';
import type { NormalizedSectionInput } from '../../lib/generated/document';
import type { UserFacingError } from '../../lib/errors';

export type ImportErrorCode = UserFacingError['code'];

export interface BeginImportRequest {
  sourcePath: string;
}

export type BeginImportOutcome =
  | { outcome: 'created'; book: BookSummary }
  | { outcome: 'duplicate'; book: BookSummary };

export interface ParsedBookMetadata {
  title: string;
  author: string | null;
  language: string | null;
}

export type ImportStage = 'copying' | 'parsing' | 'indexing';

export interface ImportEvent {
  stage: ImportStage;
  completed: number;
  total: number;
  messageKey: string;
}

export interface ParseContext {
  bookId: string;
  format: BookFormat;
  source: ArrayBuffer;
  signal: AbortSignal;
}

export interface ParserSink {
  begin(metadata: ParsedBookMetadata): Promise<void>;
  append(sections: NormalizedSectionInput[]): Promise<void>;
  progress(event: ImportEvent): void;
  writeDerivedText(name: 'document.html', content: string): Promise<void>;
}

export interface DocumentParser {
  readonly format: BookFormat;
  parse(context: ParseContext, sink: ParserSink): Promise<void>;
}

/**
 * Parser ordinals are deterministic inputs to UUID v5: section ordinals start
 * at zero across the book; block ordinals start at zero within each section.
 */
export const PARSER_ORDINAL_ORIGIN = 0 as const;
