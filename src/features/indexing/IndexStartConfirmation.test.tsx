import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';

import { IndexStartConfirmation } from './IndexStartConfirmation';

const request = {
  runId: null,
  bookId: '11111111-1111-4111-8111-111111111111',
  sourceSha256: 'a'.repeat(64),
  providerProfileId: '22222222-2222-4222-822222222222',
  pages: [
    { pageNumber: 3, qualityReason: 'no_text' as const, localTextSha256: null },
  ],
};

describe('IndexStartConfirmation', () => {
  it('uses one fresh confirmation token for one explicit start and keeps the preference profile-scoped', async () => {
    const user = userEvent.setup();
    const confirmOperation = vi
      .fn()
      .mockResolvedValue('33333333-3333-4333-833333333333');
    const createRun = vi
      .fn()
      .mockResolvedValue('44444444-4444-4444-844444444444');
    const setNoPrompt = vi.fn().mockResolvedValue(undefined);
    const started = vi.fn();
    render(
      <IndexStartConfirmation
        request={request}
        profileName="Vision profile"
        modelName="Model X"
        api={{ confirmOperation, createRun }}
        onSetProfileNoPrompt={setNoPrompt}
        onStarted={started}
      />,
    );

    expect(
      screen.getByText(/Profile: Vision profile. Model: Model X. Pages: 1/),
    ).toBeVisible();
    expect(screen.getByText(/local page images/)).toBeVisible();
    await user.click(screen.getByRole('checkbox'));
    await user.click(screen.getByRole('button', { name: 'Confirm and start' }));

    expect(confirmOperation).toHaveBeenCalledWith(request);
    expect(setNoPrompt).toHaveBeenCalledWith(request.providerProfileId);
    expect(createRun).toHaveBeenCalledWith(
      '33333333-3333-4333-833333333333',
      request,
    );
    expect(started).toHaveBeenCalledWith('44444444-4444-4444-844444444444');
  });
});
