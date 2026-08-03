import AxeBuilder from '@axe-core/playwright';
import { expect, test, type Page } from '@playwright/test';

const bookId = '00000000-0000-4000-8000-000000000001';

async function installShellMock(page: Page) {
  await page.addInitScript(
    ({ fixtureBookId }) => {
      type UiLanguage = 'zh-CN' | 'zh-TW' | 'en';
      const languageKey = '__textbooklens_e2e_sqlite_language';
      const settings = (language: UiLanguage, hintCompleted = false) => ({
        onboardingCompleted: false,
        activeProviderProfileId: null,
        theme: 'system',
        contextMode: 'standard',
        uiLanguage: language,
        uiLanguageInitialized: true,
        firstReaderHintCompleted: hintCompleted,
      });
      const currentLanguage = (): UiLanguage =>
        (sessionStorage.getItem(languageKey) as UiLanguage | null) ?? 'zh-CN';
      const testWindow = window as unknown as {
        __TAURI_INTERNALS__: {
          invoke(
            command: string,
            payload?: Record<string, unknown>,
          ): Promise<unknown>;
        };
        __e2eReaderOpenCount: number;
      };
      testWindow.__e2eReaderOpenCount = 0;
      testWindow.__TAURI_INTERNALS__ = {
        async invoke(command, payload) {
          if (command === 'get_app_settings')
            return settings(currentLanguage());
          if (command === 'initialize_ui_language') {
            const detected = payload?.detected as UiLanguage;
            sessionStorage.setItem(languageKey, detected);
            return settings(detected);
          }
          if (command === 'update_ui_language') {
            const language = payload?.language as UiLanguage;
            sessionStorage.setItem(languageKey, language);
            return settings(language);
          }
          if (command === 'complete_first_reader_hint')
            return settings(currentLanguage(), true);
          if (command === 'list_books') return [];
          if (command === 'get_reader_settings')
            return {
              fontScale: 1,
              lineHeight: 1.6,
              readerWidth: 72,
              pdfZoom: 1,
              theme: 'system',
            };
          if (command === 'update_reader_settings') return payload?.settings;
          if (command === 'get_reader_bootstrap')
            return {
              book: {
                id: fixtureBookId,
                title: 'Synthetic Reader Fixture',
                originalFilename: 'fixture.docx',
                author: null,
                language: null,
                format: 'docx',
                importStatus: 'ready',
                importErrorCode: null,
                importErrorMessage: null,
                importErrorStage: null,
                readingProgress: 0,
                createdAt: '2026-08-03T00:00:00Z',
                updatedAt: '2026-08-03T00:00:00Z',
                lastOpenedAt: null,
              },
              lastLocator: {
                format: 'docx',
                startBlockId: 'block-1',
                startOffset: 0,
                endBlockId: 'block-1',
                endOffset: 0,
              },
            };
          if (command === 'list_reader_sections')
            return [
              {
                id: 'section-1',
                parentId: null,
                ordinal: 0,
                title: 'Synthetic section',
                locator: {
                  format: 'docx',
                  startBlockId: 'block-1',
                  startOffset: 0,
                  endBlockId: 'block-1',
                  endOffset: 0,
                },
              },
            ];
          if (command === 'read_derived_text') {
            testWindow.__e2eReaderOpenCount += 1;
            return '<p data-block-id="block-1">Synthetic reader content</p>';
          }
          if (command === 'list_annotation_markers') return [];
          if (command === 'search_book') return [];
          throw new Error(`Unexpected IPC command: ${command}`);
        },
      };

      let fullscreenElement: Element | null = null;
      Object.defineProperty(document, 'fullscreenElement', {
        configurable: true,
        get: () => fullscreenElement,
      });
      Object.defineProperty(Element.prototype, 'requestFullscreen', {
        configurable: true,
        value: async () => {
          fullscreenElement = document.querySelector('.reader-layout');
          document.dispatchEvent(new Event('fullscreenchange'));
        },
      });
      Object.defineProperty(document, 'exitFullscreen', {
        configurable: true,
        value: async () => {
          fullscreenElement = null;
          document.dispatchEvent(new Event('fullscreenchange'));
        },
      });
    },
    { fixtureBookId: bookId },
  );
}

test.beforeEach(async ({ page }) => {
  await installShellMock(page);
});

test('switches all three application languages immediately and restores the mock SQLite choice after restart', async ({
  page,
}) => {
  await page.goto('/library');
  const language = page.getByRole('button', { name: '应用语言' });
  await expect(language).toHaveText('简体中文');

  await language.click();
  await page.getByRole('menuitemradio', { name: '繁體中文' }).click();
  await expect(page.getByRole('button', { name: '應用程式語言' })).toHaveText(
    '繁體中文',
  );
  await expect(page.getByRole('link', { name: '教學指令' })).toBeVisible();

  await page.getByRole('button', { name: '應用程式語言' }).click();
  await page.getByRole('menuitemradio', { name: 'English' }).click();
  await expect(
    page.getByRole('link', { name: 'Teaching instructions' }),
  ).toBeVisible();
  await page.reload();
  await expect(
    page.getByRole('button', { name: 'Application language' }),
  ).toHaveText('English');

  await page.getByRole('button', { name: 'Application language' }).click();
  await page.getByRole('menuitemradio', { name: '简体中文' }).click();
  await expect(page.getByRole('link', { name: '教学指令' })).toBeVisible();
});

test('reader toolbar has only the minimal actions and transient drawers do not reopen its adapter', async ({
  page,
}) => {
  await page.goto(`/books/${bookId}/read`);
  const toolbar = page.getByRole('toolbar', { name: '阅读工具栏' });
  await expect(toolbar).toBeVisible();
  await expect(toolbar.locator(':scope > a')).toHaveCount(1);
  await expect(toolbar.locator(':scope > button')).toHaveCount(5);
  await expect(toolbar.getByRole('link', { name: '书库' })).toBeVisible();
  await expect(toolbar.getByRole('button', { name: '目录' })).toBeVisible();
  await expect(toolbar.getByRole('button', { name: '搜索' })).toBeVisible();
  await expect(toolbar.getByRole('button', { name: /框选/ })).toBeDisabled();
  await expect(toolbar.getByRole('button', { name: '阅读设置' })).toBeVisible();
  await expect(toolbar.getByRole('button', { name: '进入全屏' })).toBeVisible();
  await expect(toolbar).toContainText('Synthetic Reader Fixture');

  await toolbar.getByRole('button', { name: '目录' }).click();
  await page.getByRole('button', { name: '关闭' }).click();
  await toolbar.getByRole('button', { name: '阅读设置' }).click();
  await page.getByRole('button', { name: '关闭' }).click();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as unknown as { __e2eReaderOpenCount: number })
            .__e2eReaderOpenCount,
      ),
    )
    .toBe(1);
});

test('F11 and toolbar fullscreen stay synchronized while Escape closes the top drawer first', async ({
  page,
}) => {
  await page.goto(`/books/${bookId}/read`);
  const toolbar = page.getByRole('toolbar', { name: '阅读工具栏' });

  await page.evaluate(() =>
    window.dispatchEvent(
      new KeyboardEvent('keydown', {
        key: 'F11',
        bubbles: true,
        cancelable: true,
      }),
    ),
  );
  await expect(
    toolbar.getByRole('button', { name: '退出全屏' }),
  ).toHaveAttribute('aria-pressed', 'true');
  await toolbar.getByRole('button', { name: '搜索' }).click();
  const input = page.getByLabel('搜索书内内容');
  await expect(input).toBeFocused();
  await page.keyboard.press('Escape');
  await expect(page.getByRole('dialog', { name: '书内搜索' })).toHaveCount(0);
  await expect(toolbar.getByRole('button', { name: '搜索' })).toBeFocused();
  await expect(
    toolbar.getByRole('button', { name: '退出全屏' }),
  ).toHaveAttribute('aria-pressed', 'true');

  await toolbar.getByRole('button', { name: '退出全屏' }).click();
  await expect(
    toolbar.getByRole('button', { name: '进入全屏' }),
  ).toHaveAttribute('aria-pressed', 'false');
});

test('narrow and forced-colors reader shells retain primary actions and pass axe', async ({
  page,
}) => {
  await page.setViewportSize({ width: 360, height: 720 });
  await page.emulateMedia({
    colorScheme: 'dark',
    forcedColors: 'active',
    reducedMotion: 'reduce',
  });
  await page.goto(`/books/${bookId}/read`);
  const toolbar = page.getByRole('toolbar', { name: '阅读工具栏' });
  for (const action of ['目录', '搜索', '阅读设置', '进入全屏']) {
    const button = toolbar.getByRole('button', { name: action });
    await button.scrollIntoViewIfNeeded();
    await expect(button).toBeVisible();
  }
  await expect(toolbar.getByRole('button', { name: /框选/ })).toBeDisabled();
  const results = await new AxeBuilder({ page }).analyze();
  expect(results.violations).toEqual([]);
});
