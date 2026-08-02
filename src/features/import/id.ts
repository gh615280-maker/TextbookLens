import { v5 as uuidv5 } from 'uuid';

/** Section ordinals are zero-based and unique across the whole book. */
export function stableSectionId(
  bookId: string,
  sectionOrdinal: number,
): string {
  return uuidv5(`section:${validateOrdinal(sectionOrdinal)}`, bookId);
}

/** Block ordinals are zero-based within their containing section. */
export function stableBlockId(
  bookId: string,
  sectionOrdinal: number,
  blockOrdinal: number,
): string {
  return uuidv5(
    `block:${validateOrdinal(sectionOrdinal)}:${validateOrdinal(blockOrdinal)}`,
    bookId,
  );
}

function validateOrdinal(ordinal: number): number {
  if (!Number.isSafeInteger(ordinal) || ordinal < 0 || ordinal > 0xffff_ffff) {
    throw new TypeError('document ordinals must be unsigned 32-bit integers');
  }
  return ordinal;
}
