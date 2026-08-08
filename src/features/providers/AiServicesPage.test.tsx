import { cleanup, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { LanguageProvider, useLanguage } from '../../app/LanguageProvider';
import type { UiLanguage } from '../../lib/i18n';
import type { ProviderApi } from './api';
import { AiServicesPage } from './AiServicesPage';

const profile = {
  id: '4f9a2c86-0da8-4dd4-a255-39b4cff89c66',
  kind: 'openai' as const,
  displayName: 'Synthetic OpenAI',
  modelId: 'synthetic-text',
  contextWindowTokens: 1000,
  isActive: true,
  credentialStatus: 'available' as const,
  validatedAt: '2026-08-03T00:00:00Z',
};
const registry = {
  schemaVersion: 1,
  providers: [
    {
      kind: 'openai' as const,
      displayName: 'OpenAI',
      defaultModel: 'synthetic-text',
      fileCapabilities: {
        fileExtraction: false,
        fileOcr: false,
        maxFileBytes: null,
      },
      models: [
        {
          id: 'synthetic-text',
          displayName: 'Synthetic text',
          contextWindowTokens: 1000,
          defaultMaxOutputTokens: 100,
          textChat: 'supported' as const,
          imageInput: 'unknown' as const,
          nativePdfInput: 'unknown' as const,
          pdfInput: 'unknown' as const,
          strictStructuredOutput: 'unsupported' as const,
          imageLimits: null,
          lastVerified: '2026-08-03',
        },
      ],
    },
  ],
};
function fakeApi(): ProviderApi {
  return {
    listCapabilities: vi.fn(async () => registry),
    listProfiles: vi.fn(async () => [profile]),
    getSettings: vi.fn(async () => ({
      onboardingCompleted: false,
      activeProviderProfileId: profile.id,
      defaultLearningProfileId: profile.id,
      defaultVisionProfileId: null,
      theme: 'system' as const,
      contextMode: 'standard' as const,
      uiLanguage: 'en' as const,
      uiLanguageInitialized: true,
      firstReaderHintCompleted: false,
    })),
    validateAndSave: vi.fn(async () => profile),
    replaceCredential: vi.fn(async () => profile),
    deleteProfile: vi.fn(async () => {}),
    setDefault: vi.fn(async () => {}),
    updateConsent: vi.fn(async () => {}),
    resetConsents: vi.fn(async () => {}),
  };
}
afterEach(() => cleanup());

function languageSettings(language: UiLanguage) {
  const value = (uiLanguage: UiLanguage) => ({
    onboardingCompleted: false,
    activeProviderProfileId: null,
    defaultLearningProfileId: null,
    defaultVisionProfileId: null,
    theme: 'system' as const,
    contextMode: 'standard' as const,
    uiLanguage,
    uiLanguageInitialized: true,
    firstReaderHintCompleted: false,
  });
  return {
    getAppSettings: vi.fn(async () => value(language)),
    initializeUiLanguage: vi.fn(async (detected: UiLanguage) =>
      value(detected),
    ),
    updateUiLanguage: vi.fn(async (next: UiLanguage) => value(next)),
  };
}

function renderPage(api: ProviderApi, language: UiLanguage = 'en') {
  return render(
    <LanguageProvider api={languageSettings(language)}>
      <AiServicesPage api={api} />
    </LanguageProvider>,
  );
}

describe('AI services', () => {
  it('shows Unknown rather than treating unverified vision support as supported', async () => {
    renderPage(fakeApi());
    expect(await screen.findByText(/Vision: Unknown/)).toBeVisible();
  });
  it('keeps an invalid key only in the mounted password field and localizes the error', async () => {
    const api = fakeApi();
    api.validateAndSave = vi.fn(async () => {
      throw {
        code: 'INVALID_API_KEY',
        message: 'Invalid key',
        nextStep: 'Check it',
        diagnosticId: null,
      };
    });
    const user = userEvent.setup();
    const settings = languageSettings('en');
    function SwitchLanguage() {
      const { switchLanguage } = useLanguage();
      return (
        <button type="button" onClick={() => void switchLanguage('zh-CN')}>
          switch error language
        </button>
      );
    }
    render(
      <LanguageProvider api={settings}>
        <SwitchLanguage />
        <AiServicesPage api={api} />
      </LanguageProvider>,
    );
    const input = await screen.findByLabelText('Key');
    await user.type(input, 'synthetic-secret');
    await user.click(
      screen.getByRole('button', { name: 'Validate & Connect' }),
    );
    await waitFor(() =>
      expect(screen.getByRole('alert')).toHaveTextContent(
        'The AI service request failed.',
      ),
    );
    expect(input).toHaveValue('synthetic-secret');
    expect(screen.getByRole('alert')).not.toHaveTextContent('Invalid key');
    expect(screen.getByRole('alert')).not.toHaveTextContent('Check it');
    expect(document.body.innerHTML).not.toContain('"credential"');

    await user.click(
      screen.getByRole('button', { name: 'switch error language' }),
    );
    expect(await screen.findByRole('alert')).toHaveTextContent(
      'AI 服务请求失败。',
    );
  });

  it('updates the whole AI services surface when the application language changes', async () => {
    const settings = languageSettings('en');
    function SwitchLanguage() {
      const { switchLanguage } = useLanguage();
      return (
        <button type="button" onClick={() => void switchLanguage('zh-CN')}>
          switch language
        </button>
      );
    }
    const user = userEvent.setup();
    render(
      <LanguageProvider api={settings}>
        <SwitchLanguage />
        <AiServicesPage api={fakeApi()} />
      </LanguageProvider>,
    );
    expect(
      await screen.findByRole('heading', { name: 'AI services' }),
    ).toBeVisible();
    expect(screen.getByLabelText('Key')).toBeVisible();

    await user.click(screen.getByRole('button', { name: 'switch language' }));

    expect(
      await screen.findByRole('heading', { name: 'AI 服务' }),
    ).toBeVisible();
    expect(screen.getByLabelText('密钥')).toBeVisible();
    expect(screen.getByRole('button', { name: '设为学习服务' })).toBeVisible();
    expect(screen.queryByText('Connected profiles')).not.toBeInTheDocument();
  });
});
