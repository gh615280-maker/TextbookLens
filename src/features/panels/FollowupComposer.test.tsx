import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { FollowupComposer } from './FollowupComposer';

describe('FollowupComposer', () => {
  it('preserves a concise provider-change notice and submits bounded question text', () => {
    const onSubmit = vi.fn();
    render(
      <FollowupComposer
        providerChangeNotice="Using Current / model; history used Previous / model."
        onSubmit={onSubmit}
      />,
    );
    fireEvent.change(screen.getByRole('textbox'), {
      target: { value: ' Why? ' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Send' }));
    expect(onSubmit).toHaveBeenCalledWith('Why?');
    expect(screen.getByRole('note')).toHaveTextContent('Previous');
  });
});
