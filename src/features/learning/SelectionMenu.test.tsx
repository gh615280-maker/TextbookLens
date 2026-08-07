import { StrictMode } from 'react';
import { cleanup, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { SelectionMenu } from './SelectionMenu';
import {
  menuSnapshotFromRegion,
  menuSnapshotFromText,
} from './selection-state';

const snapshot = menuSnapshotFromText(
  {
    text: 'PRIVATE_TEXTBOOK_BODY_SENTINEL',
    anchor: {
      locator: { format: 'pdf', startPage: 1, endPage: 1, rectsByPage: null },
      quote: {
        exact: 'PRIVATE_TEXTBOOK_BODY_SENTINEL',
        prefix: '',
        suffix: '',
      },
      sectionId: '22222222-2222-4222-8222-222222222222',
    },
  },
  {
    bookId: '11111111-1111-4111-8111-111111111111',
    sectionId: '22222222-2222-4222-8222-222222222222',
    profile: { id: '33333333-3333-4333-8333-333333333333', modelId: 'model' },
    position: { x: 0, y: 0 },
  },
);

const labels = {
  menu: 'Actions',
  explain: 'Explain',
  example: 'Example',
  derive: 'Derive',
  translate: 'Translate',
  ask: 'Ask',
  note: 'Note',
  input: 'Input',
  submit: 'Send',
  unavailable: 'Unavailable',
  error: 'Error',
  noteEditor: {
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
  },
  confirmation: {
    title: 'Confirm',
    details: '{provider}/{profile}/{model}/{tokens}/{sources}/{citations}',
    noPrompt: 'Never',
    cancel: 'Cancel',
    continue: 'Continue',
    imageRisk: 'Image',
    costRisk: 'Cost',
  },
};

afterEach(cleanup);

describe('SelectionMenu', () => {
  it('keeps visual capture bytes alive through the StrictMode effect replay', async () => {
    const bytes = new Uint8Array([137, 80, 78, 71]);
    const release = vi.fn(() => bytes.fill(0));
    const visualSnapshot = menuSnapshotFromRegion(
      {
        page: 2,
        rect: { x: 0.1, y: 0.2, width: 0.3, height: 0.4 },
        sectionId: undefined,
        text: null,
        anchor: {
          kind: 'region',
          region: {
            locator: { format: 'pdf', page: 2 },
            rect: { x: 0.1, y: 0.2, width: 0.3, height: 0.4 },
            contentSha256: 'a'.repeat(64),
            textFallback: null,
          },
        },
        capture: {
          mimeType: 'image/png',
          width: 1,
          height: 1,
          bytes,
          release,
        },
      },
      {
        bookId: '11111111-1111-4111-8111-111111111111',
        sectionId: '22222222-2222-4222-8222-222222222222',
        profile: {
          id: '33333333-3333-4333-8333-333333333333',
          modelId: 'vision-model',
        },
        position: { x: 0, y: 0 },
      },
    );
    const view = render(
      <StrictMode>
        <SelectionMenu
          api={{
            prepare: vi.fn(),
            authorize: vi.fn(),
            stageRegionCapture: vi.fn(),
            discard: vi.fn(),
            invalidate: vi.fn(),
          }}
          noteApi={{
            create: vi.fn(),
            update: vi.fn(),
            delete: vi.fn(),
            get: vi.fn(),
            list: vi.fn(),
          }}
          labels={labels}
          snapshot={visualSnapshot}
          surface={{ handoff: vi.fn() }}
          onClose={vi.fn()}
          onError={vi.fn()}
        />
      </StrictMode>,
    );

    await waitFor(() => expect(screen.getByRole('menu')).toBeVisible());
    expect([...bytes]).toEqual([137, 80, 78, 71]);
    expect(release).not.toHaveBeenCalled();

    view.unmount();
    await waitFor(() => expect(release).toHaveBeenCalledOnce());
    expect([...bytes]).toEqual([0, 0, 0, 0]);
  });

  it('hands an opaque safe preparation to the injected surface exactly once', async () => {
    const api = {
      prepare: vi.fn(async () => ({
        preparationId: '44444444-4444-4444-8444-444444444444',
        providerDisplayName: 'Provider',
        profileDisplayName: 'Profile',
        modelDisplayName: 'Model',
        estimatedInputTokens: 1,
        sourceCount: 0,
        citationCount: 0,
        omittedSourceCount: 0,
        willSendImage: false,
        riskFlags: [],
        requiresBlockingConfirmation: false,
        expiresAt: '2026-08-05T00:00:00Z',
        actionCategory: 'explain' as const,
      })),
      authorize: vi.fn(),
      stageRegionCapture: vi.fn(),
      discard: vi.fn(),
      invalidate: vi.fn(),
    };
    const surface = { handoff: vi.fn() };
    const noteApi = {
      create: vi.fn(),
      update: vi.fn(),
      delete: vi.fn(),
      get: vi.fn(),
      list: vi.fn(),
    };
    render(
      <SelectionMenu
        api={api}
        noteApi={noteApi}
        labels={labels}
        snapshot={snapshot}
        surface={surface}
        onClose={vi.fn()}
        onError={vi.fn()}
      />,
    );
    await userEvent.click(screen.getByRole('menuitem', { name: 'Explain' }));
    await waitFor(() => expect(surface.handoff).toHaveBeenCalledOnce());
    expect(JSON.stringify(surface.handoff.mock.calls[0])).not.toContain(
      'PRIVATE_TEXTBOOK_BODY_SENTINEL',
    );
    expect(api.prepare).toHaveBeenCalledOnce();
  });

  it('silently authorizes and stages a consented visual capture before handoff', async () => {
    const bytes = new Uint8Array([137, 80, 78, 71]);
    const visualSnapshot = menuSnapshotFromRegion(
      {
        page: 100,
        rect: { x: 0.1, y: 0.2, width: 0.3, height: 0.4 },
        text: '!',
        anchor: {
          kind: 'region',
          region: {
            locator: { format: 'pdf', page: 100 },
            rect: { x: 0.1, y: 0.2, width: 0.3, height: 0.4 },
            contentSha256: 'a'.repeat(64),
            textFallback: { exact: '!', prefix: '', suffix: '' },
          },
        },
        capture: {
          mimeType: 'image/png',
          width: 1,
          height: 1,
          bytes,
          release: vi.fn(() => bytes.fill(0)),
        },
      },
      {
        bookId: '11111111-1111-4111-8111-111111111111',
        sectionId: '22222222-2222-4222-8222-222222222222',
        profile: {
          id: '33333333-3333-4333-8333-333333333333',
          modelId: 'vision-model',
        },
        position: { x: 0, y: 0 },
      },
    );
    const api = {
      prepare: vi.fn(async () => ({
        preparationId: '44444444-4444-4444-8444-444444444444',
        providerDisplayName: 'Provider',
        profileDisplayName: 'Profile',
        modelDisplayName: 'Vision model',
        estimatedInputTokens: 1,
        sourceCount: 1,
        citationCount: 1,
        omittedSourceCount: 0,
        willSendImage: true,
        riskFlags: ['image_send' as const],
        requiresBlockingConfirmation: false,
        expiresAt: '2026-08-05T00:00:00Z',
        actionCategory: 'explain' as const,
      })),
      authorize: vi.fn(async () => '55555555-5555-4555-8555-555555555555'),
      stageRegionCapture: vi.fn(async () => {}),
      discard: vi.fn(),
      invalidate: vi.fn(),
    };
    const surface = { handoff: vi.fn(async () => {}) };
    render(
      <SelectionMenu
        api={api}
        noteApi={{
          create: vi.fn(),
          update: vi.fn(),
          delete: vi.fn(),
          get: vi.fn(),
          list: vi.fn(),
        }}
        labels={labels}
        snapshot={visualSnapshot}
        surface={surface}
        onClose={vi.fn()}
        onError={vi.fn()}
      />,
    );

    await userEvent.click(screen.getByRole('menuitem', { name: 'Explain' }));
    await waitFor(() => expect(surface.handoff).toHaveBeenCalledOnce());
    expect(api.authorize).toHaveBeenCalledWith(
      '44444444-4444-4444-8444-444444444444',
      'allow',
    );
    expect(api.stageRegionCapture).toHaveBeenCalledOnce();
    expect(api.authorize.mock.invocationCallOrder[0]).toBeLessThan(
      api.stageRegionCapture.mock.invocationCallOrder[0],
    );
    expect(api.stageRegionCapture.mock.invocationCallOrder[0]).toBeLessThan(
      surface.handoff.mock.invocationCallOrder[0],
    );
  });

  it('creates a note through the local API without preparation or surface handoff', async () => {
    const api = {
      prepare: vi.fn(),
      authorize: vi.fn(),
      stageRegionCapture: vi.fn(),
      discard: vi.fn(),
      invalidate: vi.fn(),
    };
    const note = {
      id: '55555555-5555-4555-8555-555555555555',
      bookId: snapshot.bookId,
      sectionId: snapshot.sectionId,
      anchor: snapshot.anchor,
      selectedText: snapshot.selectedText,
      noteText: 'local note',
      revision: 1,
      createdAt: '2026-08-05T00:00:00.000Z',
      updatedAt: '2026-08-05T00:00:00.000Z',
    };
    const noteApi = {
      create: vi.fn(async () => note),
      update: vi.fn(),
      delete: vi.fn(),
      get: vi.fn(),
      list: vi.fn(),
    };
    const surface = { handoff: vi.fn() };
    const onNoteSaved = vi.fn();
    render(
      <SelectionMenu
        api={api}
        labels={labels}
        noteApi={noteApi}
        snapshot={snapshot}
        surface={surface}
        onClose={vi.fn()}
        onError={vi.fn()}
        onNoteSaved={onNoteSaved}
      />,
    );
    await userEvent.click(screen.getByRole('menuitem', { name: 'Note' }));
    await userEvent.type(
      screen.getByRole('textbox', { name: 'Personal note' }),
      'local note',
    );
    await userEvent.click(screen.getByRole('button', { name: 'Save note' }));
    await waitFor(() => expect(noteApi.create).toHaveBeenCalledOnce());
    expect(api.prepare).not.toHaveBeenCalled();
    expect(surface.handoff).not.toHaveBeenCalled();
    expect(onNoteSaved).toHaveBeenCalledWith(note);
  });
});
