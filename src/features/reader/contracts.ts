import type { UserFacingError } from '../../lib/errors';
import type { BookFormat } from '../../lib/generated/book';
import type { DocumentLocator, TextQuote } from '../../lib/generated/document';

export type ReaderSource =
  | { kind: 'document_bytes'; bytes: ArrayBuffer }
  | { kind: 'sanitized_html'; html: string };

export interface SelectionSnapshot {
  text: string;
  anchor: { locator: DocumentLocator; quote: TextQuote; sectionId: string | null };
}

export interface ReadingProgress {
  fraction: number;
  locator: DocumentLocator | null;
}

export interface NavigationResult {
  found: boolean;
}

export interface AnnotationMarker {
  id: string;
  label: string;
}

export interface ReaderSearchHit {
  locator: DocumentLocator;
  text: string;
}

export interface ReaderAdapterEvents {
  onSelection(snapshot: SelectionSnapshot | null): void;
  onProgress(progress: ReadingProgress): void;
  onMarkerActivate(annotationId: string): void;
  onFailure(error: UserFacingError): void;
}

export interface ReaderAdapter {
  readonly format: BookFormat;
  open(source: ReaderSource, initial?: DocumentLocator | null): Promise<void>;
  getSelectionSnapshot(): SelectionSnapshot | null;
  navigate(locator: DocumentLocator): Promise<NavigationResult>;
  showAnnotations(items: AnnotationMarker[]): Promise<void>;
  search(query: string): Promise<ReaderSearchHit[]>;
  getProgress(): ReadingProgress;
  dispose(): void;
}

export type ReaderAdapterFactory = (events: ReaderAdapterEvents) => ReaderAdapter;
