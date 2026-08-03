import { cleanup, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it } from 'vitest';

import { LanguageProvider } from '../app/LanguageProvider';
import type { AppSettingsDto } from '../lib/generated/settings';
import { LanguageMenu } from './LanguageMenu';

afterEach(cleanup);

const settings: AppSettingsDto = {
  onboardingCompleted: false,
  activeProviderProfileId: null,
  theme: 'system',
  contextMode: 'standard',
  uiLanguage: 'zh-TW',
  uiLanguageInitialized: true,
  firstReaderHintCompleted: false,
};

describe('LanguageMenu', () => {
  it('uses self-named, keyboard-selectable menu items with current-item semantics', async () => {
    const user = userEvent.setup();
    render(
      <LanguageProvider
        api={{
          getAppSettings: async () => settings,
          initializeUiLanguage: async () => settings,
          updateUiLanguage: async (language) => ({
            ...settings,
            uiLanguage: language,
          }),
        }}
      >
        <LanguageMenu />
      </LanguageProvider>,
    );

    const trigger = await screen.findByRole('button', {
      name: '應用程式語言',
    });
    expect(trigger).toHaveTextContent('繁體中文');
    await user.click(trigger);

    expect(screen.getByRole('menu')).toBeVisible();
    expect(
      screen.getByRole('menuitemradio', { name: '繁體中文' }),
    ).toHaveAttribute('aria-checked', 'true');
    expect(
      screen.getByRole('menuitemradio', { name: '简体中文' }),
    ).toBeVisible();
    expect(
      screen.getByRole('menuitemradio', { name: 'English' }),
    ).toBeVisible();

    await user.keyboard('{ArrowDown}{Enter}');
    expect(
      await screen.findByRole('button', { name: 'Application language' }),
    ).toHaveTextContent('English');
  });
});
