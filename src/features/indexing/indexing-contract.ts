import type {
  IndexPageStatus,
  IndexQualityReason,
} from '../../lib/generated/indexing';

/**
 * Versioned, local-only result used to seed a future index run.  It deliberately
 * contains neither a document source nor PDF.js objects.
 */
export interface LocalPdfPageQualityDto {
  readonly schemaVersion: 1;
  readonly pageNumber: number;
  readonly qualityReason: IndexQualityReason;
  readonly status: Extract<IndexPageStatus, 'not_required' | 'needs_review'>;
}

/** Metadata for a bounded local page capture. Bytes are held only until submit or cancellation. */
export interface RenderedPdfPageDto {
  readonly schemaVersion: 1;
  readonly mimeType: 'image/png';
  readonly width: number;
  readonly height: number;
  readonly decodedPixelCount: number;
  readonly encodedByteLength: number;
  readonly sha256: string;
  readonly bytes: Uint8Array;
}
