import { describe, expect, it } from 'vitest';

import {
  clampMenuPosition,
  menuSnapshotFromRegion,
  menuSnapshotFromText,
  preparationMetadata,
} from './selection-state';

const ids = {
  bookId: '11111111-1111-4111-8111-111111111111',
  sectionId: '22222222-2222-4222-8222-222222222222',
  profile: { id: '33333333-3333-4333-8333-333333333333', modelId: 'model' },
};

describe('learning selection state', () => {
  it('freezes text and reliable-region selections into the same menu model', () => {
    const text = menuSnapshotFromText(
      {
        text: 'frozen selection',
        anchor: {
          locator: {
            format: 'pdf',
            startPage: 1,
            endPage: 1,
            rectsByPage: null,
          },
          quote: { exact: 'frozen selection', prefix: '', suffix: '' },
          sectionId: ids.sectionId,
        },
      },
      { ...ids, position: { x: 10, y: 20 } },
    );
    const region = menuSnapshotFromRegion(
      {
        rect: { x: 0, y: 0, width: 1, height: 1 },
        text: 'frozen selection',
        capture: null,
        anchor: {
          kind: 'region',
          region: {
            locator: { format: 'pdf', page: 1 },
            rect: { x: 0, y: 0, width: 1, height: 1 },
            contentSha256: 'a'.repeat(64),
            textFallback: { exact: 'frozen selection', prefix: '', suffix: '' },
          },
        },
      },
      { ...ids, position: { x: 10, y: 20 } },
    );
    expect(Object.isFrozen(text)).toBe(true);
    expect(text.selectedText).toBe(region.selectedText);
    expect(region.contentKind).toBe('reliable_text_region');
    expect(preparationMetadata(text, 'explain', null).selectedText).toBe(
      'frozen selection',
    );
  });

  it('clamps a copied screen position without changing its snapshot', () => {
    expect(
      clampMenuPosition(
        { x: 99, y: 99 },
        { width: 30, height: 40 },
        {
          width: 100,
          height: 100,
          offsetLeft: 0,
          offsetTop: 0,
        },
      ),
    ).toEqual({ x: 70, y: 60 });
  });
});
