import { EpubCFI } from 'epubjs';
import type { ReaderSection } from './api';
import type { RegionSelectionResult, SelectionSnapshot } from './contracts';

export function sectionIdForTextSelection(
  selection: Readonly<SelectionSnapshot>,
  sections: readonly ReaderSection[],
): string | undefined {
  const supplied = selection.anchor.sectionId;
  if (selection.anchor.locator.format === 'epub') {
    return resolveEpubSection(sections, selection.anchor.locator.cfi, supplied);
  }
  if (supplied && sections.some((section) => section.id === supplied)) {
    return supplied;
  }
  const locator = selection.anchor.locator;
  if (locator.format === 'pdf') {
    return sectionIdForPdfRange(sections, locator.startPage, locator.endPage);
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
    region.anchor.kind === 'region' &&
    region.anchor.region.locator.format === 'epub'
  ) {
    return resolveEpubSection(
      sections,
      region.anchor.region.locator.cfi,
      region.sectionId,
    );
  }
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
  return locator.format === 'docx'
    ? sectionIdForDocxBlock(sections, locator.blockId)
    : undefined;
}

/** CFI identifies the original spine; import ordinals may omit image-only entries. */
export function sectionIdForEpubCfi(
  sections: readonly Pick<ReaderSection, 'id' | 'locator'>[],
  cfi: string,
): string | undefined {
  const spine = epubSpineIndex(cfi);
  if (spine === undefined) return undefined;
  const matches = sections.filter(
    (section) =>
      section.locator.format === 'epub' &&
      epubSpineIndex(section.locator.cfi) === spine,
  );
  return matches.length === 1 ? matches[0]!.id : undefined;
}

function resolveEpubSection(
  sections: readonly ReaderSection[],
  cfi: string,
  supplied: string | null | undefined,
): string | undefined {
  const resolved = sectionIdForEpubCfi(sections, cfi);
  if (!resolved) return undefined;
  if (supplied?.startsWith('spine-')) {
    return supplied === `spine-${epubSpineIndex(cfi)}` ? resolved : undefined;
  }
  return !supplied || supplied === resolved ? resolved : undefined;
}

function epubSpineIndex(cfi: string): number | undefined {
  if (!cfi.startsWith('epubcfi(') || !cfi.endsWith(')')) return undefined;
  try {
    const parsed = new EpubCFI(cfi);
    return Number.isSafeInteger(parsed.spinePos) && parsed.spinePos >= 0
      ? parsed.spinePos
      : undefined;
  } catch {
    return undefined;
  }
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
