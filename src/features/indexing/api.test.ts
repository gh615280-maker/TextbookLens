import { afterEach, describe, expect, it } from 'vitest';

import { clearMocks, installTauriMock } from '../../test/tauri-mock';
import { TauriIndexingApi } from './api';

const BOOK_ID = '22222222-2222-4222-8222-222222222222';
const RUN_ID = '11111111-1111-4111-8111-111111111111';

afterEach(clearMocks);

describe('TauriIndexingApi.findCurrentRunForBook', () => {
  it('uses a canonical book UUID and preserves a missing durable run as null', async () => {
    const calls = installTauriMock((command) => {
      expect(command).toBe('find_current_index_run_for_book');
      return null;
    });

    await expect(
      new TauriIndexingApi().findCurrentRunForBook(BOOK_ID),
    ).resolves.toBeNull();
    expect(calls).toHaveLength(1);
    expect(calls[0]?.payload).toEqual({ bookId: BOOK_ID });
  });

  it('rejects a client-derived non-UUID before invoking Rust', async () => {
    const calls = installTauriMock(() => {
      throw new Error('must not invoke');
    });

    await expect(
      new TauriIndexingApi().findCurrentRunForBook(`${BOOK_ID}/${RUN_ID}`),
    ).rejects.toThrow();
    expect(calls).toHaveLength(0);
  });
});
