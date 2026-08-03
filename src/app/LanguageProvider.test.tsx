import { cleanup, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it } from 'vitest';

import type { AppSettingsDto } from '../lib/generated/settings';
import { LanguageProvider, useLanguage } from './LanguageProvider';

afterEach(cleanup);

const settings = (overrides: Partial<AppSettingsDto> = {}): AppSettingsDto => ({
  onboardingCompleted: false,
  activeProviderProfileId: null,
  theme: 'system',
  contextMode: 'standard',
  uiLanguage: 'en',
  uiLanguageInitialized: false,
  firstReaderHintCompleted: false,
  ...overrides,
});

function Consumer() {
  const { message, switchLanguage, uiLanguage } = useLanguage();
  return (
    <>
      <p>{message('nav.library')}</p>
      <p>{uiLanguage}</p>
      <button type="button" onClick={() => void switchLanguage('zh-TW')}>
        Switch
      </button>
    </>
  );
}

describe('LanguageProvider', () => {
  it('detects and persists the Windows language only when initialization is pending', async () => {
    const calls: string[] = [];
    render(
      <LanguageProvider
        api={{
          getAppSettings: async () => settings(),
          initializeUiLanguage: async (language) => {
            calls.push(language);
            return settings({
              uiLanguage: language,
              uiLanguageInitialized: true,
            });
          },
          updateUiLanguage: async (language) =>
            settings({ uiLanguage: language }),
        }}
        detectedLanguages={['zh-Hant-HK']}
      >
        <Consumer />
      </LanguageProvider>,
    );

    expect(await screen.findByText('圖書館')).toBeVisible();
    expect(calls).toEqual(['zh-TW']);
  });

  it('rolls an optimistic language switch back and announces a safe error', async () => {
    const user = userEvent.setup();
    render(
      <LanguageProvider
        api={{
          getAppSettings: async () =>
            settings({ uiLanguage: 'en', uiLanguageInitialized: true }),
          initializeUiLanguage: async (language) =>
            settings({ uiLanguage: language }),
          updateUiLanguage: async () =>
            Promise.reject(new Error('database unavailable')),
        }}
      >
        <Consumer />
      </LanguageProvider>,
    );

    expect(await screen.findByText('Library')).toBeVisible();
    await user.click(screen.getByRole('button', { name: 'Switch' }));
    await waitFor(() => expect(screen.getByText('en')).toBeVisible());
    expect(screen.getByRole('status')).toHaveTextContent(
      'Unable to change the application language.',
    );
  });
});
