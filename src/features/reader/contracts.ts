import type { UserFacingError } from '../../lib/errors';
import type { BookFormat } from '../../lib/generated/book';
import type {
  ContentAnchor,
  DocumentLocator,
  NormalizedRect,
  TextQuote,
} from '../../lib/generated/document';

export type ReaderSource =
  | { kind: 'document_bytes'; bytes: ArrayBuffer }
  | { kind: 'sanitized_html'; html: string };

export interface SelectionSnapshot {
  text: string;
  anchor: {
    locator: DocumentLocator;
    quote: TextQuote;
    sectionId: string | null;
  };
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
  kind: 'ai_conversation' | 'note';
  label: string;
  /** Display-only anchor supplied by a later persistence phase; adapters never rewrite it. */
  anchor?: SelectionSnapshot['anchor'];
  relocationStatus: MarkerRelocationStatus;
}

export type MarkerRelocationStatus = 'primary' | 'fallback' | 'unresolved';
export interface MarkerRelocation {
  annotationId: string;
  relocationStatus: MarkerRelocationStatus;
}

export interface ReaderSearchHit {
  locator: DocumentLocator;
  text: string;
}

export type PdfRegionInstructionCode =
  | 'pdf_region_cross_page'
  | 'pdf_region_too_small'
  | 'pdf_region_cancelled'
  | 'pdf_region_unavailable';

export interface RegionSelectionOptions {
  /** Required before any mixed/visual pixels are materialized. */
  confirmVisualCapture(): boolean | Promise<boolean>;
}

export interface RegionSelectionResult {
  page: number;
  rect: NormalizedRect;
  text: string | null;
  anchor: ContentAnchor;
  capture: {
    mimeType: 'image/png';
    width: number;
    height: number;
    bytes: Uint8Array;
    release(): void;
  } | null;
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
  showAnnotations(items: AnnotationMarker[]): Promise<MarkerRelocation[]>;
  search(query: string): Promise<ReaderSearchHit[]>;
  getProgress(): ReadingProgress;
  beginRegionSelection?(
    options: RegionSelectionOptions,
  ): Promise<RegionSelectionResult | null>;
  cancel?(): void;
  cancelRegionSelection?(): void;
  dispose(): void;
}

export type ReaderAdapterFactory = (
  events: ReaderAdapterEvents,
) => ReaderAdapter;
