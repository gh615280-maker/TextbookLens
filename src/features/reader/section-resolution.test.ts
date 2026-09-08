import { describe, expect, it } from 'vitest';

import type { ReaderSection } from './api';
import {
  sectionIdForRegionSelection,
  sectionIdForTextSelection,
} from './section-resolution';

const sections: ReaderSection[] = [
  section('11111111-1111-4111-8111-111111111111', 13),
  section('22222222-2222-4222-8222-222222222222', 14),
];

describe('reader section resolution', () => {
  it('resolves skipped EPUB spine entries by CFI instead of the normalized ordinal', () => {
    const epubSections: ReaderSection[] = [12, 20, 38].map((step, ordinal) => ({
      id: `00000000-0000-4000-8000-00000000000${ordinal + 1}`,
      parentId: null,
      ordinal,
      title: `Synthetic chapter ${ordinal}`,
      locator: {
        format: 'epub',
        cfi: `epubcfi(/6/${step}[chapter-${ordinal}]!/4/2)`,
        sectionId: `00000000-0000-4000-8000-00000000000${ordinal + 1}`,
      },
    }));
    const locator = {
      format: 'epub' as const,
      cfi: 'epubcfi(/6/38[chapter-2]!/4/94,/1:0,/1:9)',
      sectionId: 'spine-18',
    };
    expect(
      sectionIdForTextSelection(
        {
          text: 'selection',
          anchor: {
            locator,
            sectionId: 'spine-18',
            quote: { exact: 'selection', prefix: '', suffix: '' },
          },
        },
        epubSections,
      ),
    ).toBe(epubSections[2]!.id);
    expect(
      sectionIdForRegionSelection(
        {
          sectionId: 'spine-18',
          cfi: locator.cfi,
          rect: { x: 0, y: 0, width: 0.5, height: 0.5 },
          text: 'selection',
          capture: null,
          anchor: {
            kind: 'region',
            region: {
              locator,
              rect: { x: 0, y: 0, width: 0.5, height: 0.5 },
              contentSha256: 'a'.repeat(64),
              textFallback: null,
            },
          },
        },
        epubSections,
      ),
    ).toBe(epubSections[2]!.id);
    // Normalized ordinal zero is not the unindexed image-only spine zero.
    expect(
      sectionIdForTextSelection(
        {
          text: 'selection',
          anchor: {
            locator: {
              ...locator,
              cfi: 'epubcfi(/6/2!/4/2)',
              sectionId: 'spine-0',
            },
            sectionId: 'spine-0',
            quote: { exact: 'selection', prefix: '', suffix: '' },
          },
        },
        epubSections,
      ),
    ).toBeUndefined();
  });

  it('does not accept an EPUB section ID that contradicts its CFI', () => {
    const id = '33333333-3333-4333-8333-333333333333';
    const epub: ReaderSection = {
      id,
      parentId: null,
      ordinal: 0,
      title: 'Synthetic',
      locator: { format: 'epub', cfi: 'epubcfi(/6/12!/4/2)', sectionId: id },
    };
    expect(
      sectionIdForTextSelection(
        {
          text: 'selection',
          anchor: {
            sectionId: id,
            locator: {
              format: 'epub',
              cfi: 'epubcfi(/6/38!/4/2)',
              sectionId: id,
            },
            quote: { exact: 'selection', prefix: '', suffix: '' },
          },
        },
        [epub],
      ),
    ).toBeUndefined();
  });

  it('maps PDF text and region selections to the section owning their page', () => {
    expect(
      sectionIdForTextSelection(
        {
          text: 'selection',
          anchor: {
            locator: {
              format: 'pdf',
              startPage: 14,
              endPage: 14,
              rectsByPage: null,
            },
            quote: { exact: 'selection', prefix: '', suffix: '' },
            sectionId: null,
          },
        },
        sections,
      ),
    ).toBe(sections[1]?.id);
    expect(
      sectionIdForRegionSelection(
        {
          page: 14,
          rect: { x: 0.1, y: 0.1, width: 0.2, height: 0.2 },
          text: null,
          capture: null,
          anchor: {
            kind: 'region',
            region: {
              locator: { format: 'pdf', page: 14 },
              rect: { x: 0.1, y: 0.1, width: 0.2, height: 0.2 },
              contentSha256: 'a'.repeat(64),
              textFallback: null,
            },
          },
        },
        sections,
      ),
    ).toBe(sections[1]?.id);
  });

  it('does not silently bind an unindexed PDF page to the first section', () => {
    expect(
      sectionIdForTextSelection(
        {
          text: 'selection',
          anchor: {
            locator: {
              format: 'pdf',
              startPage: 2,
              endPage: 2,
              rectsByPage: null,
            },
            quote: { exact: 'selection', prefix: '', suffix: '' },
            sectionId: null,
          },
        },
        sections,
      ),
    ).toBeUndefined();
  });

  it('maps rendition EPUB ordinals and stable DOCX block boundaries to persisted sections', () => {
    const epub = {
      id: '33333333-3333-4333-8333-333333333333',
      parentId: null,
      ordinal: 2,
      title: 'EPUB section',
      locator: {
        format: 'epub' as const,
        cfi: 'epubcfi(/6/6!/4/2)',
        sectionId: '33333333-3333-4333-8333-333333333333',
      },
    };
    const docx = {
      id: '44444444-4444-4444-8444-444444444444',
      parentId: null,
      ordinal: 3,
      title: 'DOCX section',
      locator: {
        format: 'docx' as const,
        startBlockId: 'first-block',
        startOffset: 0,
        endBlockId: 'last-block',
        endOffset: 0,
      },
    };

    expect(
      sectionIdForTextSelection(
        {
          text: 'selection',
          anchor: {
            locator: {
              format: 'epub',
              cfi: 'epubcfi(/6/6!/4/2)',
              sectionId: 'spine-2',
            },
            quote: { exact: 'selection', prefix: '', suffix: '' },
            sectionId: 'spine-2',
          },
        },
        [epub, docx],
      ),
    ).toBe(epub.id);
    expect(
      sectionIdForRegionSelection(
        {
          blockId: 'first-block',
          rect: { x: 0.1, y: 0.1, width: 0.2, height: 0.2 },
          text: 'selection',
          capture: null,
          anchor: {
            kind: 'region',
            region: {
              locator: { format: 'docx', blockId: 'first-block' },
              rect: { x: 0.1, y: 0.1, width: 0.2, height: 0.2 },
              contentSha256: 'a'.repeat(64),
              textFallback: null,
            },
          },
        },
        [epub, docx],
      ),
    ).toBe(docx.id);
  });
});

function section(id: string, page: number): ReaderSection {
  return {
    id,
    parentId: null,
    ordinal: page,
    title: `Page ${page}`,
    locator: {
      format: 'pdf',
      startPage: page,
      endPage: page,
      rectsByPage: null,
    },
  };
}
