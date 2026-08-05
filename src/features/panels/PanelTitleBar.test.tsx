import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { PanelTitleBar } from './PanelTitleBar';

describe('PanelTitleBar', () => {
  it('only starts drag from the title bar and exposes collapse state', () => {
    const onDragStart = vi.fn();
    render(
      <PanelTitleBar
        collapsed={false}
        status="streaming"
        title="Source"
        onCollapse={vi.fn()}
        onDragStart={onDragStart}
        onHide={vi.fn()}
      />,
    );
    fireEvent.pointerDown(screen.getByLabelText('Move learning panel'));
    expect(onDragStart).toHaveBeenCalledOnce();
    expect(screen.getByRole('button', { name: 'Collapse' })).toHaveAttribute(
      'aria-expanded',
      'true',
    );
  });
});
