import { afterEach, describe, expect, it } from 'vitest';

import { clearMocks, installTauriMock } from '../../test/tauri-mock';
import { TauriNotesApi, type CreateNoteInput } from './api';

const BOOK_ID = '11111111-1111-4111-8111-111111111111';
const SECTION_ID = '22222222-2222-4222-8222-222222222222';
const NOTE_ID = '33333333-3333-4333-8333-333333333333';
const BODY_SENTINEL = 'PRIVATE_NOTE_BODY_SENTINEL';

afterEach(() => clearMocks());

describe('TauriNotesApi', () => {
  it('normalizes line endings, validates the anchor contract, and freezes the DTO', async () => {
    let actualPayload: unknown;
    const calls = installTauriMock((command, payload) => {
      expect(command).toBe('create_note');
      actualPayload = payload;
      return note({ noteText: '  first\nsecond  ' });
    });
    const created = await new TauriNotesApi().create({
      ...createInput(),
      noteText: '  first\r\nsecond\r  ',
    });

    expect(created.noteText).toBe('  first\nsecond  ');
    expect(Object.isFrozen(created)).toBe(true);
    expect(Object.isFrozen(created.anchor)).toBe(true);
    expect(actualPayload).toMatchObject({ noteText: '  first\nsecond\n  ' });
    expect(calls).toHaveLength(1);
    expect(JSON.stringify(calls[0]?.payload)).not.toContain(BODY_SENTINEL);
    expect(calls[0]?.payload).toMatchObject({
      hasNoteText: true,
      hasSelectedText: true,
      anchorKind: 'text',
    });
  });

  it('rejects empty, over-bound, mismatched, and extra input before IPC', async () => {
    const calls = installTauriMock(() => note());
    const api = new TauriNotesApi();
    const invalid = [
      { ...createInput(), noteText: ' \n\t ' },
      { ...createInput(), noteText: 'x'.repeat(16_385) },
      { ...createInput(), selectedText: 'different' },
      { ...createInput(), unexpectedPrompt: BODY_SENTINEL },
    ];
    for (const input of invalid) {
      await expect(api.create(input as CreateNoteInput)).rejects.toEqual({
        code: 'INVALID_INPUT',
      });
    }
    expect(calls).toHaveLength(0);
  });

  it('maps invocation and malformed response failures to stable body-free codes', async () => {
    installTauriMock(() => {
      throw {
        code: 'REQUEST_CONFLICT',
        message: BODY_SENTINEL,
        nextStep: BODY_SENTINEL,
      };
    });
    const api = new TauriNotesApi();
    await expect(
      api.update({
        bookId: BOOK_ID,
        noteId: NOTE_ID,
        expectedRevision: 1,
        noteText: BODY_SENTINEL,
      }),
    ).rejects.toEqual({ code: 'REQUEST_CONFLICT' });

    installTauriMock(() => ({ ...note(), noteText: '' }));
    await expect(api.get(BOOK_ID, NOTE_ID)).rejects.toEqual({
      code: 'DATABASE_ERROR',
    });
  });
});

function createInput(): CreateNoteInput {
  return {
    bookId: BOOK_ID,
    sectionId: SECTION_ID,
    anchor: {
      kind: 'text',
      selection: {
        locator: {
          format: 'pdf',
          startPage: 1,
          endPage: 1,
          rectsByPage: null,
        },
        quote: { exact: 'selected text', prefix: '', suffix: '' },
        sectionId: SECTION_ID,
      },
    },
    selectedText: 'selected text',
    noteText: BODY_SENTINEL,
  };
}

function note(overrides: Record<string, unknown> = {}) {
  return {
    id: NOTE_ID,
    bookId: BOOK_ID,
    sectionId: SECTION_ID,
    anchor: createInput().anchor,
    selectedText: 'selected text',
    noteText: BODY_SENTINEL,
    revision: 1,
    createdAt: '2026-08-05T00:00:00.000Z',
    updatedAt: '2026-08-05T00:00:00.000Z',
    ...overrides,
  };
}
