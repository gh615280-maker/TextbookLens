import type { IndexQualityReason } from '../../lib/generated/indexing';
import type { PdfTextItem } from '../import/parsers/pdf-layout';

import type { LocalPdfPageQualityDto } from './indexing-contract';

export const PDF_QUALITY_RULE_VERSION = 1 as const;

const MIN_MEANINGFUL_CHARACTERS = 24;
const MIN_TEXT_COVERAGE = 0.007;
const MAX_REPLACEMENT_OR_CONTROL_RATIO = 0.12;
const MIN_REPLACEMENT_OR_CONTROL_CHARACTERS = 4;
const MAX_DUPLICATE_GLYPH_RATIO = 0.92;
const MIN_DUPLICATE_GLYPHS = 80;
const MIN_CONTRADICTORY_OVERLAP_RATIO = 0.8;

export interface PdfPageQualityInput {
  readonly pageNumber: number;
  readonly width: number;
  readonly height: number;
  readonly items: readonly PdfTextItem[];
}

/**
 * Classifies extracted PDF.js text with versioned, deterministic local rules.
 * It has no model input and intentionally returns only a safe page-level DTO.
 */
export function assessPdfPageQuality(
  input: PdfPageQualityInput,
): LocalPdfPageQualityDto {
  const reason = qualityReason(input);
  return {
    schemaVersion: PDF_QUALITY_RULE_VERSION,
    pageNumber: input.pageNumber,
    qualityReason: reason,
    status: reason === 'reliable_text' ? 'not_required' : 'needs_review',
  };
}

function qualityReason(input: PdfPageQualityInput): IndexQualityReason {
  const text = input.items.map((item) => item.str).join('');
  const characters = [...text];
  const meaningful = characters.filter((character) =>
    /[\p{L}\p{N}]/u.test(character),
  );
  if (meaningful.length === 0) return 'no_text';

  if (
    textCoverage(input) < MIN_TEXT_COVERAGE ||
    meaningful.length < MIN_MEANINGFUL_CHARACTERS
  )
    return 'very_low_text_coverage';

  const suspiciousCharacters = characters.filter(
    (character) => character === '\ufffd' || /[\p{Cc}\p{Cs}]/u.test(character),
  );
  if (
    suspiciousCharacters.length >= MIN_REPLACEMENT_OR_CONTROL_CHARACTERS &&
    suspiciousCharacters.length / characters.length >
      MAX_REPLACEMENT_OR_CONTROL_RATIO
  ) {
    return 'high_replacement_or_control_ratio';
  }

  const glyphs = meaningful.map((character) => character.normalize('NFC'));
  const glyphCounts = new Map<string, number>();
  for (const glyph of glyphs)
    glyphCounts.set(glyph, (glyphCounts.get(glyph) ?? 0) + 1);
  const highestGlyphCount = [...glyphCounts.values()].reduce(
    (highest, count) => {
      return Math.max(highest, count);
    },
    0,
  );
  if (
    glyphs.length >= MIN_DUPLICATE_GLYPHS &&
    highestGlyphCount / glyphs.length >= MAX_DUPLICATE_GLYPH_RATIO
  ) {
    return 'extreme_duplicate_glyphs';
  }

  if (hasLayoutContradiction(input.items)) return 'layout_contradiction';
  return 'reliable_text';
}

function textCoverage({ width, height, items }: PdfPageQualityInput): number {
  if (
    !Number.isFinite(width) ||
    !Number.isFinite(height) ||
    width <= 0 ||
    height <= 0
  )
    return 0;
  const pageArea = width * height;
  const occupiedArea = items.reduce((total, item) => {
    const itemHeight = Math.hypot(
      item.transform[2] ?? 0,
      item.transform[3] ?? 0,
    );
    const itemWidth = Math.abs(item.width);
    if (!Number.isFinite(itemWidth) || !Number.isFinite(itemHeight))
      return total;
    return total + itemWidth * itemHeight;
  }, 0);
  return Math.min(1, occupiedArea / pageArea);
}

function hasLayoutContradiction(items: readonly PdfTextItem[]): boolean {
  for (let leftIndex = 0; leftIndex < items.length; leftIndex += 1) {
    const left = itemBounds(items[leftIndex]);
    if (!left) continue;
    for (
      let rightIndex = leftIndex + 1;
      rightIndex < items.length;
      rightIndex += 1
    ) {
      const right = itemBounds(items[rightIndex]);
      if (
        !right ||
        normalizeItemText(items[leftIndex]) ===
          normalizeItemText(items[rightIndex])
      )
        continue;
      const intersectionWidth = Math.max(
        0,
        Math.min(left.right, right.right) - Math.max(left.left, right.left),
      );
      const intersectionHeight = Math.max(
        0,
        Math.min(left.top, right.top) - Math.max(left.bottom, right.bottom),
      );
      const intersection = intersectionWidth * intersectionHeight;
      const smallerArea = Math.min(left.area, right.area);
      if (
        smallerArea > 0 &&
        intersection / smallerArea >= MIN_CONTRADICTORY_OVERLAP_RATIO
      )
        return true;
    }
  }
  return false;
}

function itemBounds(item: PdfTextItem | undefined) {
  if (!item) return null;
  const [, , transformWidth, transformHeight, x, y] = item.transform;
  const height = Math.hypot(transformWidth ?? 0, transformHeight ?? 0);
  const width = Math.abs(item.width);
  if (
    ![x, y, width, height].every(Number.isFinite) ||
    width <= 0 ||
    height <= 0
  )
    return null;
  return {
    left: x,
    right: x + width,
    bottom: y,
    top: y + height,
    area: width * height,
  };
}

function normalizeItemText(item: PdfTextItem | undefined): string {
  return item?.str.replace(/\s+/gu, ' ').trim() ?? '';
}
