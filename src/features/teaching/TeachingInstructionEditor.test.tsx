import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';

import { TeachingInstructionEditor } from './TeachingInstructionEditor';

describe('TeachingInstructionEditor', () => {
  it('counts Unicode scalar values and exposes draft and save states', async () => {
    const onChange = vi.fn();
    const onSave = vi.fn();
    const user = userEvent.setup();
    const { rerender } = render(
      <TeachingInstructionEditor
        draft="A😀"
        dirty
        onChange={onChange}
        onClear={vi.fn()}
        onDefault={vi.fn()}
        onSave={onSave}
        stage="saved"
      />,
    );

    expect(screen.getByRole('status')).toHaveTextContent('Unsaved changes');
    expect(screen.getByText('2 / 1000')).toBeVisible();
    await user.click(screen.getByRole('button', { name: 'Save' }));
    expect(onSave).toHaveBeenCalledOnce();
    await user.type(screen.getByLabelText('Instruction'), 'x');
    expect(onChange).toHaveBeenCalledWith('A😀x');

    rerender(
      <TeachingInstructionEditor
        draft=""
        dirty={false}
        onChange={onChange}
        onClear={vi.fn()}
        onDefault={vi.fn()}
        onSave={onSave}
        stage="saving"
      />,
    );
    expect(screen.getByRole('status')).toHaveTextContent('Saving');
    expect(screen.getByRole('button', { name: 'Save' })).toBeDisabled();
  });
});
