import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { NoteEditor, type NoteEditorLabels } from './NoteEditor';
import type { Note, NotesApi } from './api';

const BOOK_ID = '11111111-1111-4111-8111-111111111111';
const SECTION_ID = '22222222-2222-4222-8222-222222222222';
const NOTE_ID = '33333333-3333-4333-8333-333333333333';
const labels: NoteEditorLabels = {
  input: 'Personal note',
  save: 'Save note',
  cancel: 'Cancel note',
  empty: 'Empty note',
  tooLong: 'Note too long',
  error: 'Note error',
  conflict: 'Note conflict',
  reload: 'Reload note',
  preserve: 'Keep draft',
  delete: 'Delete note',
  deleteConfirm: 'Delete this note?',
  confirmDelete: 'Delete',
  cancelDelete: 'Keep note',
};
const anchor = {
  kind: 'text' as const,
  selection: {
    locator: {
      format: 'pdf' as const,
      startPage: 1,
      endPage: 1,
      rectsByPage: null,
    },
    quote: { exact: 'selected text', prefix: '', suffix: '' },
    sectionId: SECTION_ID,
  },
};

afterEach(cleanup);

describe('NoteEditor', () => {
  it('gives stable accessible feedback for empty and over-bound drafts', async () => {
    const api = mockApi();
    renderEditor(api);
    await userEvent.click(screen.getByRole('button', { name: 'Save note' }));
    expect(screen.getByRole('status')).toHaveTextContent('Empty note');
    fireEvent.change(screen.getByRole('textbox', { name: 'Personal note' }), {
      target: { value: 'x'.repeat(16_385) },
    });
    await userEvent.click(screen.getByRole('button', { name: 'Save note' }));
    expect(screen.getByRole('status')).toHaveTextContent('Note too long');
    expect(api.create).not.toHaveBeenCalled();
  });

  it('submits one local request, ignores duplicates, and does not revive after unmount', async () => {
    let finish!: (value: Readonly<Note>) => void;
    const api = mockApi();
    api.create.mockImplementation(
      () => new Promise((resolve) => (finish = resolve)),
    );
    const onSaved = vi.fn();
    const view = renderEditor(api, { onSaved });
    await userEvent.type(
      screen.getByRole('textbox', { name: 'Personal note' }),
      'draft body',
    );
    const save = screen.getByRole('button', { name: 'Save note' });
    await userEvent.click(save);
    await userEvent.click(save);
    expect(api.create).toHaveBeenCalledOnce();
    expect(api.update).not.toHaveBeenCalled();
    expect(api.delete).not.toHaveBeenCalled();
    view.unmount();
    finish(note({ noteText: 'draft body' }));
    await Promise.resolve();
    expect(onSaved).not.toHaveBeenCalled();
  });

  it('offers explicit reload or draft preservation after a revision conflict', async () => {
    const api = mockApi();
    api.update.mockRejectedValueOnce({ code: 'REQUEST_CONFLICT' });
    api.get.mockResolvedValue(
      note({ noteText: 'saved elsewhere', revision: 2 }),
    );
    renderEditor(api, { existing: note() });
    const input = screen.getByRole('textbox', { name: 'Personal note' });
    await userEvent.clear(input);
    await userEvent.type(input, 'my draft');
    await userEvent.click(screen.getByRole('button', { name: 'Save note' }));
    expect(screen.getByRole('status')).toHaveTextContent('Note conflict');
    expect(screen.getByRole('button', { name: 'Reload note' })).toBeVisible();
    expect(screen.getByRole('button', { name: 'Keep draft' })).toBeVisible();

    await userEvent.click(screen.getByRole('button', { name: 'Keep draft' }));
    expect(input).toHaveValue('my draft');
    expect(api.get).toHaveBeenCalledOnce();
    api.update.mockResolvedValueOnce(
      note({ noteText: 'my draft', revision: 3 }),
    );
    await userEvent.click(screen.getByRole('button', { name: 'Save note' }));
    expect(api.update).toHaveBeenLastCalledWith(
      expect.objectContaining({ expectedRevision: 2, noteText: 'my draft' }),
    );
  });

  it('requires explicit delete confirmation and reports deletion once', async () => {
    const api = mockApi();
    const onDeleted = vi.fn();
    renderEditor(api, { existing: note(), onDeleted });
    await userEvent.click(screen.getByRole('button', { name: 'Delete note' }));
    expect(api.delete).not.toHaveBeenCalled();
    expect(screen.getByRole('alertdialog')).toHaveTextContent(
      'Delete this note?',
    );
    await userEvent.click(screen.getByRole('button', { name: 'Delete' }));
    await waitFor(() => expect(onDeleted).toHaveBeenCalledOnce());
    expect(api.delete).toHaveBeenCalledWith({
      bookId: BOOK_ID,
      noteId: NOTE_ID,
      expectedRevision: 1,
    });
  });
});

function renderEditor(
  api: ReturnType<typeof mockApi>,
  options: {
    existing?: Readonly<Note>;
    onSaved?(note: Readonly<Note>): void;
    onDeleted?(): void;
  } = {},
) {
  return render(
    <NoteEditor
      api={api}
      anchor={anchor}
      bookId={BOOK_ID}
      labels={labels}
      note={options.existing}
      sectionId={SECTION_ID}
      selectedText="selected text"
      onCancel={vi.fn()}
      onDeleted={options.onDeleted}
      onSaved={options.onSaved ?? vi.fn()}
    />,
  );
}

function mockApi() {
  return {
    create: vi.fn<NotesApi['create']>(async () => note()),
    update: vi.fn<NotesApi['update']>(async () => note({ revision: 2 })),
    delete: vi.fn<NotesApi['delete']>(async () => {}),
    get: vi.fn<NotesApi['get']>(async () => note()),
    list: vi.fn<NotesApi['list']>(async () => [note()]),
  };
}

function note(overrides: Partial<Note> = {}): Readonly<Note> {
  return Object.freeze({
    id: NOTE_ID,
    bookId: BOOK_ID,
    sectionId: SECTION_ID,
    anchor,
    selectedText: 'selected text',
    noteText: 'original note',
    revision: 1,
    createdAt: '2026-08-05T00:00:00.000Z',
    updatedAt: '2026-08-05T00:00:00.000Z',
    ...overrides,
  });
}
