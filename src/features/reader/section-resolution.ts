import type { ReaderSection } from './api';
import type { RegionSelectionResult, SelectionSnapshot } from './contracts';

export function sectionIdForTextSelection(
  selection: Readonly<SelectionSnapshot>,
  sections: readonly ReaderSection[],
): string | undefined {
  const supplied = selection.anchor.sectionId;
  if (supplied && sections.some((section) => section.id === supplied)) {
    return supplied;
  }
  const locator = selection.anchor.locator;
  if (locator.format === 'pdf') {
    return sectionIdForPdfRange(sections, locator.startPage, locator.endPage);
  }
  if (locator.format === 'epub') {
    return sectionIdForEpubRendition(sections, supplied);
  }
  return locator.format === 'docx'
    ? sectionIdForDocxBlock(sections, locator.startBlockId)
    : undefined;
}

export function sectionIdForRegionSelection(
  region: Readonly<RegionSelectionResult>,
  sections: readonly ReaderSection[],
): string | undefined {
  if (
    region.sectionId &&
    sections.some((section) => section.id === region.sectionId)
  ) {
    return region.sectionId;
  }
  const anchor = region.anchor;
  if (anchor.kind !== 'region') return undefined;
  const locator = anchor.region.locator;
  if (locator.format === 'pdf') {
    return sectionIdForPdfRange(sections, locator.page, locator.page);
  }
  if (locator.format === 'epub') {
    return sectionIdForEpubRendition(sections, region.sectionId);
  }
  return sectionIdForDocxBlock(sections, locator.blockId);
}

function sectionIdForEpubRendition(
  sections: readonly ReaderSection[],
  renditionSectionId: string | null | undefined,
): string | undefined {
  const match = /^spine-(\d+)$/u.exec(renditionSectionId ?? '');
  if (!match) return undefined;
  const ordinal = Number(match[1]);
  return sections.find(
    (section) =>
      section.locator.format === 'epub' && section.ordinal === ordinal,
  )?.id;
}

function sectionIdForDocxBlock(
  sections: readonly ReaderSection[],
  blockId: string,
): string | undefined {
  return sections.find(
    (section) =>
      section.locator.format === 'docx' &&
      (section.locator.startBlockId === blockId ||
        section.locator.endBlockId === blockId),
  )?.id;
}

function sectionIdForPdfRange(
  sections: readonly ReaderSection[],
  startPage: number,
  endPage: number,
): string | undefined {
  return sections.find(
    (section) =>
      section.locator.format === 'pdf' &&
      startPage >= section.locator.startPage &&
      endPage <= section.locator.endPage,
  )?.id;
}
