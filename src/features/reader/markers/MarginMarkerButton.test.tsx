import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';

import { MarginMarkerButton } from './MarginMarkerButton';

describe('MarginMarkerButton', () => {
  it('distinguishes AI and note markers without relying on color and activates only the annotation ID', async () => {
    const user = userEvent.setup();
    const onActivate = vi.fn();
    render(
      <>
        <MarginMarkerButton
          id="ai-id"
          kind="ai_conversation"
          onActivate={onActivate}
        />
        <MarginMarkerButton id="note-id" kind="note" onActivate={onActivate} />
      </>,
    );

    const ai = screen.getByRole('button', {
      name: 'View AI conversation marker',
    });
    const note = screen.getByRole('button', {
      name: 'View personal note marker',
    });
    expect(ai).toHaveAttribute('data-marker-shape', 'speech');
    expect(note).toHaveAttribute('data-marker-shape', 'note');
    expect(ai).toHaveAttribute('data-marker-pattern', 'stripes');
    expect(note).toHaveAttribute('data-marker-pattern', 'dots');
    expect(ai.querySelector('.margin-marker-dot')).toBeInTheDocument();
    await user.click(note);
    expect(onActivate).toHaveBeenCalledWith('note-id');
    expect(onActivate).toHaveBeenCalledWith(expect.any(String));
  });
});
