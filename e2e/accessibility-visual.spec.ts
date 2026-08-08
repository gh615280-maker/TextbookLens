import AxeBuilder from '@axe-core/playwright';
import { expect, test, type Page } from '@playwright/test';

import { messageCatalogs } from '../src/lib/i18n';

const BOOK_ID = '10000000-0000-4000-8000-000000000004';
const RUN_ID = '20000000-0000-4000-8000-000000000004';
const FIXTURE_TIME = '2026-08-08T00:00:00.000Z';

const routeHeadings = [
  ['/onboarding', 'Get started'],
  ['/library', 'Library'],
  ['/teaching-instructions', 'Teaching instructions'],
  ['/ai-services', 'AI services'],
  [`/books/${BOOK_ID}/overview`, 'Learning overview'],
  ['/settings', 'Settings'],
  [`/books/${BOOK_ID}/index-quality/${RUN_ID}`, 'Index quality'],
] as const;

test.beforeEach(async ({ page }) => {
  await blockExternalNetwork(page);
  await installAuditMock(page, 'en');
});

test.afterEach(async ({ page }) => {
  const external = await page.evaluate(
    () =>
      (window as unknown as { __a11yExternalRequests?: string[] })
        .__a11yExternalRequests ?? [],
  );
  expect(external).toEqual([]);
});

test('three catalogs have exact keys, placeholders, and no silent English copies', () => {
  const catalogs = messageCatalogs();
  const keys = Object.keys(catalogs.en).sort();
  const shared = new Set([
    'language.zhCN',
    'language.zhTW',
    'language.en',
    'indexQuality.review.latex',
  ]);
  const placeholders = (value: string) =>
    [...value.matchAll(/\{(\w+)\}/g)].map((match) => match[1]).sort();

  for (const catalog of Object.values(catalogs)) {
    expect(Object.keys(catalog).sort()).toEqual(keys);
    for (const key of keys) {
      const typedKey = key as keyof typeof catalog;
      expect(catalog[typedKey].trim(), key).not.toHaveLength(0);
      expect(placeholders(catalog[typedKey]), key).toEqual(
        placeholders(catalogs.en[typedKey]),
      );
    }
  }
  for (const key of keys) {
    if (shared.has(key)) continue;
    const typedKey = key as keyof typeof catalogs.en;
    expect(
      new Set([
        catalogs.en[typedKey],
        catalogs['zh-CN'][typedKey],
        catalogs['zh-TW'][typedKey],
      ]).size,
      key,
    ).toBeGreaterThan(1);
  }
});

test('every primary route has named landmarks and passes axe with synthetic data', async ({
  page,
}) => {
  for (const [route, heading] of routeHeadings) {
    await page.goto(route);
    await expect(
      page.getByRole('heading', { level: 1, name: heading }),
    ).toBeVisible();
    await expect(page.getByRole('main')).toBeVisible();
    const results = await new AxeBuilder({ page }).analyze();
    expect(results.violations, route).toEqual([]);
  }

  await page.goto(`/books/${BOOK_ID}/read`);
  await expect(
    page.getByRole('main', { name: 'Reading content' }),
  ).toBeVisible();
  await expect(
    page.getByRole('toolbar', { name: 'Reader toolbar' }),
  ).toBeVisible();
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
});

test('locale switching covers route, dialog, menu, and error-free index surfaces', async ({
  page,
}) => {
  await page.goto(`/books/${BOOK_ID}/index-quality/${RUN_ID}`);
  await expect(
    page.getByRole('heading', { name: 'Index quality' }),
  ).toBeVisible();

  await switchLanguage(page, '简体中文');
  await expect(page.getByRole('heading', { name: '索引质量' })).toBeVisible();
  await expect(page.getByText('Index quality')).toHaveCount(0);

  await switchLanguage(page, '繁體中文');
  await expect(page.getByRole('heading', { name: '索引品質' })).toBeVisible();
  await expect(page.getByText('Index quality')).toHaveCount(0);

  await page.goto('/onboarding');
  await expect(page.getByRole('heading', { name: '選擇教材' })).toBeVisible();
  await expect(page.getByRole('button', { name: '匯入教材' })).toBeVisible();

  await switchLanguage(page, 'English');
  await page.goto('/settings');
  const backupTrigger = page.getByRole('button', { name: 'Create backup' });
  await backupTrigger.click();
  const dialog = page.getByRole('dialog', { name: 'Create local backup' });
  await expect(dialog).toBeVisible();
  const dialogButtons = dialog.getByRole('button');
  await dialogButtons.last().focus();
  await page.keyboard.press('Tab');
  await expect(dialogButtons.first()).toBeFocused();
  await page.keyboard.press('Escape');
  await expect(dialog).toHaveCount(0);
  await expect(backupTrigger).toBeFocused();
});

test('keyboard, forced colors, reduced motion, viewports, scale proxies, and pseudo-long text remain operable', async ({
  page,
}) => {
  await page.goto(`/books/${BOOK_ID}/read`);
  await page.keyboard.press('Tab');
  const skip = page.getByRole('link', { name: 'Skip to main content' });
  await expect(skip).toBeFocused();
  await page.keyboard.press('Enter');
  await expect(
    page.getByRole('main', { name: 'Reading content' }),
  ).toBeFocused();

  const searchTrigger = page.getByRole('button', { name: 'Search' });
  await searchTrigger.click();
  const searchDialog = page.getByRole('dialog', { name: 'Search this book' });
  await expect(searchDialog).toHaveAttribute('aria-modal', 'true');
  const focusable = searchDialog.locator(
    'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])',
  );
  await focusable.last().focus();
  await page.keyboard.press('Tab');
  await expect(focusable.first()).toBeFocused();
  await focusable.first().focus();
  await page.keyboard.press('Shift+Tab');
  await expect(focusable.last()).toBeFocused();
  await page.keyboard.press('Escape');
  await expect(searchDialog).toHaveCount(0);
  await expect(searchTrigger).toBeFocused();

  await page.emulateMedia({
    colorScheme: 'dark',
    forcedColors: 'active',
    reducedMotion: 'reduce',
  });
  await page.reload();
  await expect(
    page.getByRole('toolbar', { name: 'Reader toolbar' }),
  ).toBeVisible();
  const forcedColorAudit = await page.evaluate(() => {
    const body = getComputedStyle(document.body);
    const sample = document.querySelector<HTMLElement>('.reader-layout');
    const motion = getComputedStyle(sample ?? document.body);
    return {
      bodyBackground: body.backgroundColor,
      animationDuration: motion.animationDuration,
      transitionDuration: motion.transitionDuration,
    };
  });
  expect(forcedColorAudit.bodyBackground).not.toBe('rgba(0, 0, 0, 0)');
  expect(
    cssTimeInMilliseconds(forcedColorAudit.animationDuration),
  ).toBeLessThanOrEqual(0.011);
  expect(
    cssTimeInMilliseconds(forcedColorAudit.transitionDuration),
  ).toBeLessThanOrEqual(0.011);
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);

  await page.emulateMedia({
    forcedColors: 'none',
    reducedMotion: 'no-preference',
  });
  for (const width of [320, 480, 768, 1024, 1440]) {
    await page.setViewportSize({ width, height: width < 768 ? 720 : 900 });
    await page.goto('/library');
    await expectNoDocumentOverflow(page, `viewport ${width}`);
    await expect(page.getByRole('heading', { name: 'Library' })).toBeVisible();
  }

  for (const [scale, logicalWidth] of [
    [125, 1152],
    [150, 960],
    [200, 720],
  ] as const) {
    await page.setViewportSize({ width: logicalWidth, height: 720 });
    await page.goto('/settings');
    await expectNoDocumentOverflow(page, `${scale}% logical viewport proxy`);
    await expect(
      page.getByRole('button', { name: 'Create backup' }),
    ).toBeVisible();
  }

  await page.setViewportSize({ width: 320, height: 720 });
  await page.goto('/library');
  await page.evaluate(() => {
    for (const element of document.querySelectorAll<HTMLElement>(
      'main h1, main button, main label, main a',
    )) {
      const text = element.textContent?.trim();
      if (text)
        element.textContent = `⟦${text} — PSEUDO LONG STRING PSEUDO LONG STRING⟧`;
    }
  });
  await expectNoDocumentOverflow(page, 'pseudo-long string at 320px');
});

async function switchLanguage(page: Page, target: string) {
  await page
    .getByRole('button', {
      name: /Application language|应用语言|應用程式語言/u,
    })
    .click();
  await page.getByRole('menuitemradio', { name: target }).click();
}

function cssTimeInMilliseconds(value: string) {
  const amount = Number.parseFloat(value);
  return value.endsWith('ms') ? amount : amount * 1000;
}

async function expectNoDocumentOverflow(page: Page, label: string) {
  const dimensions = await page.evaluate(() => ({
    client: document.documentElement.clientWidth,
    scroll: document.documentElement.scrollWidth,
  }));
  expect(dimensions.scroll, label).toBeLessThanOrEqual(dimensions.client + 1);
}

async function blockExternalNetwork(page: Page) {
  await page.addInitScript(() => {
    (
      window as unknown as { __a11yExternalRequests: string[] }
    ).__a11yExternalRequests = [];
  });
  await page.route('**/*', async (route) => {
    const url = new URL(route.request().url());
    if (
      url.protocol === 'data:' ||
      url.protocol === 'blob:' ||
      url.hostname === '127.0.0.1' ||
      url.hostname === 'localhost'
    ) {
      await route.continue();
      return;
    }
    await page.evaluate((blocked) => {
      (
        window as unknown as { __a11yExternalRequests: string[] }
      ).__a11yExternalRequests.push(blocked);
    }, url.href);
    await route.abort('blockedbyclient');
  });
}

async function installAuditMock(
  page: Page,
  initialLanguage: 'zh-CN' | 'zh-TW' | 'en',
) {
  await page.addInitScript(
    ({ bookId, runId, time, initial }) => {
      type UiLanguage = 'zh-CN' | 'zh-TW' | 'en';
      const storageKey = '__textbooklens_p15t4_language';
      if (!sessionStorage.getItem(storageKey))
        sessionStorage.setItem(storageKey, initial);
      const language = () => sessionStorage.getItem(storageKey) as UiLanguage;
      const settings = () => ({
        onboardingCompleted: true,
        activeProviderProfileId: null,
        defaultLearningProfileId: null,
        defaultVisionProfileId: null,
        theme: 'system',
        contextMode: 'standard',
        uiLanguage: language(),
        uiLanguageInitialized: true,
        firstReaderHintCompleted: true,
      });
      const book = {
        id: bookId,
        title: 'Synthetic accessibility textbook',
        originalFilename: 'synthetic-accessibility.docx',
        author: 'Synthetic fixture',
        language: 'en',
        format: 'docx',
        importStatus: 'ready',
        importErrorCode: null,
        importErrorMessage: null,
        importErrorStage: null,
        readingProgress: 0,
        fullTextQaReady: false,
        indexAggregate: {
          status: 'partial',
          totalPages: 2,
          indexedPages: 1,
          reviewPages: 0,
          failedPages: 0,
        },
        createdAt: time,
        updatedAt: time,
        lastOpenedAt: null,
      };
      const registry = {
        schemaVersion: 1,
        providers: [
          {
            kind: 'openai',
            displayName: 'Synthetic provider',
            defaultModel: 'synthetic-model',
            models: [
              {
                id: 'synthetic-model',
                displayName: 'Synthetic model',
                contextWindowTokens: 4096,
                defaultMaxOutputTokens: 512,
                textChat: 'supported',
                imageInput: 'unsupported',
                pdfInput: 'unsupported',
                strictStructuredOutput: 'supported',
                imageLimits: null,
                lastVerified: '2026-08-08',
              },
            ],
          },
        ],
      };
      const testWindow = window as unknown as {
        __TAURI_INTERNALS__: {
          invoke(
            command: string,
            payload?: Record<string, unknown>,
          ): Promise<unknown>;
        };
      };
      testWindow.__TAURI_INTERNALS__ = {
        async invoke(command, payload = {}) {
          switch (command) {
            case 'get_app_settings':
              return settings();
            case 'initialize_ui_language':
              sessionStorage.setItem(storageKey, String(payload.detected));
              return settings();
            case 'update_ui_language':
              sessionStorage.setItem(storageKey, String(payload.language));
              return settings();
            case 'get_onboarding_state':
              return {
                step: 'book',
                selectedBook: null,
                hasReadyBook: false,
                learningProfileConnected: false,
                visionProfileConnected: false,
                localTextQuality: 'pending',
                canSkipOnboarding: window.location.pathname !== '/onboarding',
              };
            case 'list_books':
              return [];
            case 'list_provider_capabilities':
              return registry;
            case 'list_provider_profiles':
              return [];
            case 'get_teaching_instruction':
              return { instruction: '', revision: 0, updatedAt: time };
            case 'get_learning_overview':
              return {
                bookId,
                format: 'docx',
                teachingInstructionConfigured: false,
                sectionCount: 1,
                sections: [
                  {
                    id: 'section-1',
                    parentId: null,
                    ordinal: 0,
                    title: 'Synthetic section',
                    localTextItemCount: 1,
                    userNoteCount: 0,
                    completedConversationCount: 0,
                    completedExchangeCount: 0,
                  },
                ],
                sources: [],
                activity: {
                  userNoteCount: 0,
                  completedConversationCount: 0,
                  completedExchangeCount: 0,
                  citationCount: 0,
                },
              };
            case 'list_book_learning_conversation_summaries':
              return [];
            case 'get_maintenance_status':
              return { code: 'MAINTENANCE_AVAILABLE', activeOperations: [] };
            case 'get_storage_usage':
              return {
                totalBytes: 2048,
                totalFileCount: 2,
                categories: [
                  { category: 'source', bytes: 1024, fileCount: 1 },
                  { category: 'database', bytes: 1024, fileCount: 1 },
                ],
              };
            case 'get_reader_settings':
              return {
                fontScale: 1,
                lineHeight: 1.6,
                readerWidth: 72,
                pdfZoom: 1,
                theme: 'system',
              };
            case 'update_reader_settings':
              return payload.settings;
            case 'get_reader_bootstrap':
              return {
                book,
                lastLocator: {
                  format: 'docx',
                  startBlockId: 'block-1',
                  startOffset: 0,
                  endBlockId: 'block-1',
                  endOffset: 0,
                },
              };
            case 'list_reader_sections':
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
            case 'read_derived_text':
              return '<p data-block-id="block-1">Synthetic reader content only.</p>';
            case 'list_annotation_markers':
            case 'search_book':
              return [];
            case 'get_index_run_aggregate':
              return {
                runId,
                bookId,
                controlStatus: 'paused',
                aggregateStatus: 'partial',
                pages: {
                  total: 2,
                  notRequired: 0,
                  queued: 1,
                  rendering: 0,
                  sending: 0,
                  parsing: 0,
                  validating: 0,
                  indexed: 1,
                  needsReview: 0,
                  failed: 0,
                  cancelled: 0,
                },
                updatedAt: time,
              };
            case 'list_index_page_reviews':
              return [];
            default:
              throw new Error(`Unexpected P15T4 IPC command: ${command}`);
          }
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
    {
      bookId: BOOK_ID,
      runId: RUN_ID,
      time: FIXTURE_TIME,
      initial: initialLanguage,
    },
  );
}
