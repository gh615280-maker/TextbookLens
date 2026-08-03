import { cleanup, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';

import type { ProviderProfileSummary } from '../../lib/generated/provider';
import { ProviderConsentSettings } from './ProviderConsentSettings';

const profile: ProviderProfileSummary = {
  id: '4f9a2c86-0da8-4dd4-a255-39b4cff89c66',
  kind: 'openai',
  displayName: 'Synthetic OpenAI',
  modelId: 'gpt-5.6',
  contextWindowTokens: 32_000,
  isActive: true,
  credentialStatus: 'available',
  validatedAt: '2026-08-03T00:00:00Z',
};

afterEach(cleanup);

describe('ProviderConsentSettings', () => {
  it('describes prompt-only behavior and resets exactly the requested profile', async () => {
    const onReset = vi.fn(async () => {});
    const user = userEvent.setup();
    render(
      <ProviderConsentSettings
        profile={profile}
        onReset={onReset}
        busy={false}
      />,
    );

    expect(
      screen.getByText(
        /never send images, start AI indexing, or perform network work/i,
      ),
    ).toBeVisible();
    expect(onReset).not.toHaveBeenCalled();

    const trigger = screen.getByRole('button', {
      name: 'Reset consent prompts',
    });
    await user.click(trigger);
    const confirmation = screen.getByRole('button', { name: 'Reset' });
    expect(confirmation).toHaveFocus();
    await user.click(confirmation);

    expect(onReset).toHaveBeenCalledOnce();
    expect(onReset).toHaveBeenCalledWith(profile.id);
    expect(trigger).toHaveFocus();
  });
});
