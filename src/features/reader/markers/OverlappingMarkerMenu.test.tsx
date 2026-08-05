import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';

import type { AnnotationMarker } from '../contracts';
import { OverlappingMarkerMenu } from './OverlappingMarkerMenu';

const markers: AnnotationMarker[] = [
  {
    id: 'ai',
    kind: 'ai_conversation',
    conversationId: 'conversation',
    label: 'View AI conversation marker',
    relocationStatus: 'primary',
  },
  {
    id: 'note',
    kind: 'note',
    conversationId: null,
    label: 'View personal note marker',
    relocationStatus: 'primary',
  },
];

describe('OverlappingMarkerMenu', () => {
  it('exposes a keyboard-operable compact list and returns focus on close', async () => {
    const user = userEvent.setup();
    const returnFocus = document.createElement('button');
    document.body.append(returnFocus);
    returnFocus.focus();
    const activate = vi.fn();
    const close = vi.fn();
    const view = render(
      <OverlappingMarkerMenu
        markers={markers}
        returnFocus={returnFocus}
        onActivate={activate}
        onClose={close}
      />,
    );
    expect(
      screen.getByRole('dialog', { name: 'Overlapping markers' }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole('button', { name: /View AI conversation/u }),
    ).toHaveFocus();
    const noteButton = screen.getByRole('button', {
      name: /View personal note/u,
    });
    noteButton.focus();
    const replacementClose = vi.fn();
    view.rerender(
      <OverlappingMarkerMenu
        markers={markers}
        returnFocus={returnFocus}
        onActivate={activate}
        onClose={replacementClose}
      />,
    );
    expect(noteButton).toHaveFocus();
    await user.click(
      screen.getByRole('button', { name: /View personal note/u }),
    );
    expect(activate).toHaveBeenCalledWith(markers[1]);
    await user.keyboard('{Escape}');
    expect(close).not.toHaveBeenCalled();
    expect(replacementClose).toHaveBeenCalled();
    view.unmount();
    expect(returnFocus).toHaveFocus();
    returnFocus.remove();
  });
});
