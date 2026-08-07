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
  return locator.format === 'pdf'
    ? sectionIdForPdfRange(sections, locator.startPage, locator.endPage)
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
  return locator.format === 'pdf'
    ? sectionIdForPdfRange(sections, locator.page, locator.page)
    : undefined;
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
