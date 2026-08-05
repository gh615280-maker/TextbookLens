import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';

import { SelectionMenu } from './SelectionMenu';
import { menuSnapshotFromText } from './selection-state';

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

describe('SelectionMenu', () => {
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
    const surface = { handoff: vi.fn(), note: vi.fn() };
    render(
      <SelectionMenu
        api={api}
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
});
