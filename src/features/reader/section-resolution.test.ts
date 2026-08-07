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
