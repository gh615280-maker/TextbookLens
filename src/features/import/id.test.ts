import { describe, expect, it } from 'vitest';

import { stableBlockId, stableSectionId } from './id';

describe('stable import IDs', () => {
  const bookId = '4f9a2c86-0da8-4dd4-a255-39b4cff89c66';

  it('matches the Rust RFC 4122 UUID-v5 vectors', () => {
    expect(stableSectionId(bookId, 3)).toBe(
      '70c92c3b-d44a-5345-a7eb-839e79c5b322',
    );
    expect(stableBlockId(bookId, 3, 7)).toBe(
      '2af8bb3b-d9be-5fb6-9246-86a57e8eec56',
    );
  });
});
