import type {
  ContentAnchor,
  RegionAnchor,
  RegionLocator,
  TextQuote,
} from '../../../lib/generated/document';

export type { ContentAnchor, RegionAnchor, RegionLocator, TextQuote };

export interface QuoteMatch {
  startCp: number;
  endCp: number;
  startUtf16: number;
  endUtf16: number;
  confidence: 'exact_unique' | 'context_unique';
}
