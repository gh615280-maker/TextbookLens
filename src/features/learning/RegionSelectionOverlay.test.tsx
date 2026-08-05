import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { RegionSelectionOverlay } from './RegionSelectionOverlay';

describe('RegionSelectionOverlay', () => {
  it('announces an active one-shot selection and cancels on Escape', () => {
    const cancel = vi.fn();
    render(
      <RegionSelectionOverlay
        active
        instruction="Drag once"
        status="Cancel"
        onCancel={cancel}
      />,
    );
    expect(screen.getByRole('status')).toHaveTextContent('Drag once');
    fireEvent.keyDown(window, { key: 'Escape' });
    expect(cancel).toHaveBeenCalledOnce();
  });
});
