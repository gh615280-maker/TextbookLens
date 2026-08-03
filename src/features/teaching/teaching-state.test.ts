import { describe, expect, it } from 'vitest';

import { initialTeachingState, teachingReducer } from './teaching-state';

const instruction = {
  instruction: 'Saved instruction',
  revision: 2n,
  updatedAt: '2026-08-04T00:00:00Z',
};

describe('teachingReducer', () => {
  it('keeps a local draft until an exact revision save succeeds', () => {
    const loaded = teachingReducer(initialTeachingState, {
      type: 'loaded',
      instruction,
      learningProfileAvailable: true,
    });
    const dirty = teachingReducer(loaded, {
      type: 'draftChanged',
      draft: 'Unsaved draft',
    });
    const conflict = teachingReducer(dirty, {
      type: 'conflict',
      draft: dirty.draft,
    });

    expect(dirty).toMatchObject({ dirty: true, stage: 'saved' });
    expect(conflict).toMatchObject({
      draft: 'Unsaved draft',
      dirty: true,
      stage: 'conflict',
    });
    expect(conflict.persisted).toEqual(instruction);
  });

  it('uses deterministic empty clear and default drafts', () => {
    const loaded = teachingReducer(initialTeachingState, {
      type: 'loaded',
      instruction,
      learningProfileAvailable: false,
    });
    const cleared = teachingReducer(loaded, {
      type: 'draftChanged',
      draft: '',
    });
    expect(cleared.draft).toBe('');
    expect(cleared.dirty).toBe(true);
  });
});
