import { cleanup, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';

import { App } from '../app/App';
import { clearMocks, installTauriMock } from '../test/tauri-mock';

describe('AppShell', () => {
  afterEach(() => {
    cleanup();
    clearMocks();
  });

  it('renders the Phase 8 teaching workspace and the Phase 7 AI Services page', async () => {
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
      if (command === 'get_teaching_instruction') {
        return {
          instruction: '',
          revision: 0,
          updatedAt: '2026-08-04T00:00:00Z',
        };
      }
      if (command === 'list_provider_profiles') return [];
      if (command === 'plugin:event|listen') return 1;
      if (command === 'plugin:event|unlisten') return null;
      if (command === 'list_provider_capabilities') {
        return {
          schemaVersion: 1,
          providers: [
            {
              kind: 'openai',
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
                  textChat: 'supported',
                  imageInput: 'unknown',
                  nativePdfInput: 'unknown',
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
    (
      window as Window & {
        __TAURI_EVENT_PLUGIN_INTERNALS__: {
          unregisterListener(event: string, id: number): void;
        };
      }
    ).__TAURI_EVENT_PLUGIN_INTERNALS__.unregisterListener = () => {};

    const teaching = render(
      <App initialEntries={['/teaching-instructions']} />,
    );

    expect(
      await screen.findByRole('heading', { name: '教学指令' }),
    ).toBeVisible();
    expect(screen.getByRole('textbox', { name: '指令内容' })).toBeVisible();

    teaching.unmount();
    render(<App initialEntries={['/ai-services']} />);

    expect(
      await screen.findByRole('heading', { name: 'AI 服务' }),
    ).toBeVisible();
    expect(screen.getByText('尚未连接任何 AI 服务。')).toBeVisible();
  });

  it('keeps the navigation keyboard reachable in a narrow viewport', async () => {
    render(<App initialEntries={['/library']} />);

    const links = await screen.findAllByRole('link');
    expect(links.map((link) => link.textContent)).toContain('AI 服务');
    expect(screen.getByRole('link', { name: '设置' })).toBeVisible();
  });
});
