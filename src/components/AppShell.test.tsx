import { cleanup, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';

import { App } from '../app/App';
import { clearMocks, installTauriMock } from '../test/tauri-mock';

describe('AppShell', () => {
  afterEach(() => {
    cleanup();
    clearMocks();
  });

  it('renders the teaching placeholder and the Phase 7 AI Services page', async () => {
    installTauriMock((command) => {
      if (command === 'get_app_settings') {
        return {
          onboardingCompleted: false,
          activeProviderProfileId: null,
          defaultLearningProfileId: null,
          defaultVisionProfileId: null,
          theme: 'system',
          contextMode: 'standard',
          uiLanguage: 'zh-CN',
          uiLanguageInitialized: true,
          firstReaderHintCompleted: false,
        };
      }
      if (command === 'list_provider_profiles') return [];
      if (command === 'list_provider_capabilities') {
        return {
          schemaVersion: 1,
          providers: [
            {
              kind: 'openai',
              displayName: 'OpenAI',
              defaultModel: 'synthetic-text',
              models: [
                {
                  id: 'synthetic-text',
                  displayName: 'Synthetic text',
                  contextWindowTokens: 1000,
                  defaultMaxOutputTokens: 100,
                  textChat: 'supported',
                  imageInput: 'unknown',
                  pdfInput: 'unknown',
                  strictStructuredOutput: 'unsupported',
                  imageLimits: null,
                  lastVerified: '2026-08-03',
                },
              ],
            },
          ],
        };
      }
      throw new Error(`Unexpected Tauri command: ${command}`);
    });

    const teaching = render(
      <App initialEntries={['/teaching-instructions']} />,
    );

    expect(await screen.findByText('教学指令将在后续阶段实现。')).toBeVisible();

    teaching.unmount();
    render(<App initialEntries={['/ai-services']} />);

    expect(
      await screen.findByRole('heading', { name: 'AI services' }),
    ).toBeVisible();
    expect(screen.getByText('No provider is connected yet.')).toBeVisible();
  });

  it('keeps the navigation keyboard reachable in a narrow viewport', async () => {
    render(<App initialEntries={['/library']} />);

    const links = await screen.findAllByRole('link');
    expect(links.map((link) => link.textContent)).toContain('AI 服务');
    expect(screen.getByRole('link', { name: '设置' })).toBeVisible();
  });
});
