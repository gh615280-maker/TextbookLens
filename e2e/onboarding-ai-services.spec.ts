import AxeBuilder from '@axe-core/playwright';
import { expect, test, type Page } from '@playwright/test';
import { readFileSync } from 'node:fs';

const bookId = '00000000-0000-4000-8000-000000000071';
const invalidKey = 'synthetic-e2e-invalid-key';
const validOpenAiKey = 'synthetic-e2e-valid-openai-key';
const validGeminiKey = 'synthetic-e2e-valid-gemini-key';
const fixtureBytes = [...readFileSync('fixtures/textbook.docx')];

async function installPhaseSevenMock(page: Page) {
  await page.addInitScript(
    ({ bytes, fixtureBookId }) => {
      type UiLanguage = 'zh-CN' | 'zh-TW' | 'en';
      type BookStatus = 'parsing' | 'ready' | null;
      type Profile = {
        id: string;
        kind: 'openai' | 'gemini';
        displayName: string;
        modelId: string;
        contextWindowTokens: number;
        isActive: boolean;
        credentialStatus: 'available';
        validatedAt: string;
      };
      type MockState = {
        language: UiLanguage;
        bookStatus: BookStatus;
        profiles: Profile[];
        defaultLearningProfileId: string | null;
        defaultVisionProfileId: string | null;
        nextProfile: number;
        lastValidatedModel: string | null;
      };
      type TestWindow = Window & {
        __TAURI_INTERNALS__: {
          invoke(
            command: string,
            payload?: Record<string, unknown>,
          ): Promise<unknown>;
          transformCallback(callback: (value: unknown) => void): number;
          unregisterCallback(id: number): void;
        };
        __p7ReleaseImport(): void;
        __p7SeedReady(): void;
        __p7SafeSnapshot(): unknown;
      };

      const storageKey = '__textbooklens_p7_e2e_state';
      const stored = sessionStorage.getItem(storageKey);
      const mock: MockState = stored
        ? (JSON.parse(stored) as MockState)
        : {
            language: 'zh-CN',
            bookStatus: null,
            profiles: [],
            defaultLearningProfileId: null,
            defaultVisionProfileId: null,
            nextProfile: 0,
            lastValidatedModel: null,
          };
      const save = () =>
        sessionStorage.setItem(storageKey, JSON.stringify(mock));
      const settings = (hintCompleted = false) => ({
        onboardingCompleted: false,
        activeProviderProfileId: mock.defaultLearningProfileId,
        defaultLearningProfileId: mock.defaultLearningProfileId,
        defaultVisionProfileId: mock.defaultVisionProfileId,
        theme: 'system',
        contextMode: 'standard',
        uiLanguage: mock.language,
        uiLanguageInitialized: true,
        firstReaderHintCompleted: hintCompleted,
      });
      const book = (status: Exclude<BookStatus, null>) => ({
        id: fixtureBookId,
        title: 'Synthetic Phase 7 Textbook',
        originalFilename: 'textbook.docx',
        author: 'TextbookLens fixtures',
        language: 'en',
        format: 'docx',
        importStatus: status,
        importErrorCode: null,
        importErrorMessage: null,
        importErrorStage: null,
        readingProgress: 0,
        fullTextQaReady: false,
        indexAggregate: {
          status: 'not_required',
          totalPages: 0,
          indexedPages: 0,
          reviewPages: 0,
          failedPages: 0,
        },
        createdAt: '2026-08-03T00:00:00Z',
        updatedAt: '2026-08-03T00:00:00Z',
        lastOpenedAt: null,
      });
      const registry = {
        schemaVersion: 1,
        providers: [
          {
            kind: 'openai',
            displayName: 'OpenAI',
            defaultModel: 'gpt-5.6',
            models: [
              {
                id: 'gpt-5.6',
                displayName: 'GPT-5.6',
                contextWindowTokens: 1_050_000,
                defaultMaxOutputTokens: 128_000,
                textChat: 'supported',
                imageInput: 'supported',
                pdfInput: 'supported',
                strictStructuredOutput: 'supported',
                imageLimits: null,
                lastVerified: '2026-08-03',
              },
            ],
          },
          {
            kind: 'gemini',
            displayName: 'Google Gemini',
            defaultModel: 'gemini-3.6-flash',
            models: [
              {
                id: 'gemini-3.6-flash',
                displayName: 'Gemini 3.6 Flash',
                contextWindowTokens: 1_048_576,
                defaultMaxOutputTokens: 65_536,
                textChat: 'supported',
                imageInput: 'supported',
                pdfInput: 'supported',
                strictStructuredOutput: 'supported',
                imageLimits: null,
                lastVerified: '2026-08-03',
              },
            ],
          },
        ],
      };
      const profileIds = [
        '00000000-0000-4000-8000-000000000701',
        '00000000-0000-4000-8000-000000000702',
        '00000000-0000-4000-8000-000000000703',
      ];
      const profileFor = (
        kind: Profile['kind'],
        displayName: string,
        modelId: string,
      ): Profile => ({
        id: profileIds[mock.nextProfile++]!,
        kind,
        displayName,
        modelId,
        contextWindowTokens: kind === 'openai' ? 1_050_000 : 1_048_576,
        isActive: mock.defaultLearningProfileId === null,
        credentialStatus: 'available',
        validatedAt: '2026-08-04T00:00:00Z',
      });
      const onboardingState = () => {
        const selectedBook = mock.bookStatus ? book(mock.bookStatus) : null;
        const connected = mock.profiles.some(
          (profile) =>
            profile.id === mock.defaultLearningProfileId &&
            profile.credentialStatus === 'available',
        );
        const hasReadyBook = mock.bookStatus === 'ready';
        return {
          step: !selectedBook ? 'book' : connected ? 'ready' : 'provider',
          selectedBook,
          hasReadyBook,
          learningProfileConnected: connected,
          visionProfileConnected: false,
          localTextQuality: hasReadyBook
            ? 'ready'
            : selectedBook
              ? 'pending'
              : 'unavailable',
          canSkipOnboarding: hasReadyBook && connected,
        };
      };
      const setActiveFlags = () => {
        for (const profile of mock.profiles) {
          profile.isActive = profile.id === mock.defaultLearningProfileId;
        }
      };
      let releaseImport!: () => void;
      const importGate = new Promise<void>((resolve) => {
        releaseImport = resolve;
      });
      let nextCallback = 1;
      const callbacks = new Map<number, (value: unknown) => void>();
      const testWindow = window as TestWindow;
      testWindow.__p7ReleaseImport = () => releaseImport();
      testWindow.__p7SeedReady = () => {
        mock.bookStatus = 'ready';
        if (mock.profiles.length === 0) {
          const seeded = profileFor('openai', 'Synthetic OpenAI', 'gpt-5.6');
          mock.profiles.push(seeded);
          mock.defaultLearningProfileId = seeded.id;
          setActiveFlags();
        }
        save();
      };
      testWindow.__p7SafeSnapshot = () => ({
        mock,
        session: Object.fromEntries(
          Array.from({ length: sessionStorage.length }, (_, index) => {
            const key = sessionStorage.key(index)!;
            return [key, sessionStorage.getItem(key)];
          }),
        ),
        local: Object.fromEntries(
          Array.from({ length: localStorage.length }, (_, index) => {
            const key = localStorage.key(index)!;
            return [key, localStorage.getItem(key)];
          }),
        ),
        body: document.body.textContent,
      });
      testWindow.__TAURI_INTERNALS__ = {
        transformCallback(callback) {
          const id = nextCallback++;
          callbacks.set(id, callback);
          return id;
        },
        unregisterCallback(id) {
          callbacks.delete(id);
        },
        async invoke(command, payload = {}) {
          if (command === 'get_app_settings') return settings();
          if (command === 'initialize_ui_language') {
            mock.language = payload.detected as UiLanguage;
            save();
            return settings();
          }
          if (command === 'update_ui_language') {
            mock.language = payload.language as UiLanguage;
            save();
            return settings();
          }
          if (command === 'get_onboarding_state') return onboardingState();
          if (command === 'plugin:dialog|open')
            return 'C:/synthetic/textbook.docx';
          if (command === 'begin_import') {
            mock.bookStatus = 'parsing';
            save();
            return { outcome: 'created', book: book('parsing') };
          }
          if (command === 'read_book_source') return bytes;
          if (command === 'begin_parse' || command === 'write_derived_text')
            return null;
          if (command === 'append_parsed_sections') {
            await importGate;
            return null;
          }
          if (command === 'finalize_import') {
            mock.bookStatus = 'ready';
            save();
            return book('ready');
          }
          if (
            command === 'cancel_import' ||
            command === 'mark_import_failed' ||
            command === 'save_reading_progress'
          )
            return null;
          if (command === 'list_provider_capabilities') return registry;
          if (command === 'list_provider_profiles')
            return mock.profiles.map((profile) => ({ ...profile }));
          if (command === 'validate_and_save_provider_profile') {
            const request = payload.request as {
              providerKind: Profile['kind'];
              displayName: string;
              modelId: string;
              credential: string;
            };
            const candidate = request.credential;
            request.credential = '';
            if (candidate.includes('invalid')) {
              throw {
                code: 'INVALID_API_KEY',
                message: 'The synthetic key is invalid.',
                nextStep: 'Use the deterministic valid synthetic key.',
                diagnosticId: null,
              };
            }
            const provider = registry.providers.find(
              (item) => item.kind === request.providerKind,
            )!;
            const selectedModel = request.modelId || provider.defaultModel;
            if (selectedModel !== provider.defaultModel) {
              throw new Error(
                'The test accepts only the verified default model.',
              );
            }
            mock.lastValidatedModel = selectedModel;
            const created = profileFor(
              request.providerKind,
              request.displayName,
              selectedModel,
            );
            mock.profiles.push(created);
            if (mock.defaultLearningProfileId === null) {
              mock.defaultLearningProfileId = created.id;
              setActiveFlags();
            }
            save();
            return { ...created };
          }
          if (command === 'replace_provider_profile_credential') {
            const candidate = payload.credential as string;
            payload.credential = '';
            if (!candidate) throw new Error('missing synthetic replacement');
            return {
              ...mock.profiles.find(
                (profile) => profile.id === payload.profileId,
              )!,
            };
          }
          if (command === 'set_default_provider_profile') {
            if (payload.operation === 'text_learning') {
              mock.defaultLearningProfileId = payload.profileId as string;
              setActiveFlags();
            } else {
              mock.defaultVisionProfileId = payload.profileId as string;
            }
            save();
            return null;
          }
          if (command === 'delete_provider_profile') {
            const deletedId = payload.profileId as string;
            mock.profiles = mock.profiles.filter(
              (profile) => profile.id !== deletedId,
            );
            if (mock.defaultLearningProfileId === deletedId)
              mock.defaultLearningProfileId = mock.profiles[0]?.id ?? null;
            if (mock.defaultVisionProfileId === deletedId)
              mock.defaultVisionProfileId = null;
            setActiveFlags();
            save();
            return null;
          }
          if (
            command === 'update_provider_operation_consent' ||
            command === 'reset_provider_operation_consents'
          )
            return null;
          if (command === 'list_books')
            return mock.bookStatus ? [book(mock.bookStatus)] : [];
          if (command === 'get_reader_settings')
            return {
              fontScale: 1,
              lineHeight: 1.6,
              readerWidth: 72,
              pdfZoom: 1,
              theme: 'system',
            };
          if (command === 'get_reader_bootstrap')
            return {
              book: book('ready'),
              lastLocator: {
                format: 'docx',
                startBlockId: 'block-1',
                startOffset: 0,
                endBlockId: 'block-1',
                endOffset: 0,
              },
            };
          if (command === 'read_derived_text')
            return '<p data-block-id="block-1">Synthetic reader content</p>';
          if (command === 'list_reader_sections') return [];
          if (
            command === 'list_annotation_markers' ||
            command === 'search_book'
          )
            return [];
          if (command === 'complete_first_reader_hint') return settings(true);
          throw new Error(`Unexpected IPC command: ${command}`);
        },
      };
    },
    { bytes: fixtureBytes, fixtureBookId: bookId },
  );
}

function collectExternalRequests(page: Page) {
  const external = new Set<string>();
  page.on('request', (request) => {
    const url = new URL(request.url());
    if (
      !['127.0.0.1', 'localhost'].includes(url.hostname) &&
      !['data:', 'blob:'].includes(url.protocol)
    ) {
      external.add(`${url.protocol}//${url.host}`);
    }
  });
  return external;
}

async function assertSyntheticKeysAreGone(page: Page) {
  const snapshot = JSON.stringify(
    await page.evaluate(() =>
      (
        window as Window & {
          __p7SafeSnapshot(): unknown;
        }
      ).__p7SafeSnapshot(),
    ),
  );
  for (const key of [invalidKey, validOpenAiKey, validGeminiKey]) {
    expect(snapshot).not.toContain(key);
  }
}

test.beforeEach(async ({ page }) => {
  await installPhaseSevenMock(page);
});

test('book-first onboarding keeps import through invalid and valid synthetic key paths, then reads', async ({
  page,
}) => {
  const external = collectExternalRequests(page);
  await page.goto('/onboarding');
  await switchApplicationLanguageToEnglish(page);
  await expect(
    page.getByRole('heading', { name: 'Choose a book' }),
  ).toBeVisible();

  await page.getByRole('button', { name: 'Import textbook' }).click();
  await expect(
    page.getByText(/Book: Synthetic Phase 7 Textbook/),
  ).toBeVisible();
  await expect(page.getByText(/local import continues/i)).toBeVisible();
  await switchApplicationLanguageToEnglish(page);

  const key = page.getByLabel('Key');
  await key.fill(invalidKey);
  await page.getByRole('button', { name: 'Validate & Connect' }).click();
  await expect(page.getByRole('alert')).toContainText(
    'The synthetic key is invalid.',
  );
  await expect(key).toHaveValue(invalidKey);
  await expect(page.getByText(/Synthetic Phase 7 Textbook/)).toBeVisible();

  await page.getByText('Advanced model settings').click();
  await expect(page.getByLabel('Model')).toHaveValue('gpt-5.6');
  await key.fill(validOpenAiKey);
  await page.getByRole('button', { name: 'Validate & Connect' }).click();
  await expect(page.getByText('Still importing locally')).toBeVisible();
  await expect(
    page.getByRole('button', { name: 'Start reading' }),
  ).toBeDisabled();

  await page.evaluate(() =>
    (
      window as Window & {
        __p7ReleaseImport(): void;
      }
    ).__p7ReleaseImport(),
  );
  const startReading = page.getByRole('button', { name: 'Start reading' });
  await expect(startReading).toBeEnabled();
  await startReading.click();
  await expect(page).toHaveURL(new RegExp(`/books/${bookId}/read$`));
  await expect(page.getByText('Synthetic reader content')).toBeVisible();

  const state = await page.evaluate(() =>
    (
      window as Window & {
        __p7SafeSnapshot(): { mock: { lastValidatedModel: string } };
      }
    ).__p7SafeSnapshot(),
  );
  expect(state.mock.lastValidatedModel).toBe('gpt-5.6');
  await assertSyntheticKeysAreGone(page);
  expect([...external]).toEqual([]);
});

test('AI Services adds, switches, and deletes profiles without retaining credentials', async ({
  page,
}) => {
  const external = collectExternalRequests(page);
  await page.goto('/ai-services');
  await switchApplicationLanguageToEnglish(page);
  const form = page.getByRole('form', { name: 'Connect AI provider' });

  await form.getByLabel('Key').fill(validOpenAiKey);
  await form.getByRole('button', { name: 'Validate & Connect' }).click();
  const openAi = page.getByRole('article', { name: 'OpenAI' });
  await expect(openAi).toContainText('gpt-5.6');
  await expect(openAi).toContainText('Learning default');

  await form.getByLabel('Provider').selectOption('gemini');
  await form.getByLabel('Key').fill(validGeminiKey);
  await form.getByRole('button', { name: 'Validate & Connect' }).click();
  const gemini = page.getByRole('article', { name: 'Google Gemini' });
  await expect(gemini).toContainText('gemini-3.6-flash');

  await gemini.getByRole('button', { name: 'Use for learning' }).click();
  await expect(gemini).toContainText('Learning default');
  await openAi.getByRole('button', { name: 'Delete' }).click();
  const confirmation = page.getByRole('alertdialog', { name: 'Delete OpenAI' });
  await confirmation.getByRole('button', { name: 'Delete' }).click();
  await expect(page.getByRole('article', { name: 'OpenAI' })).toHaveCount(0);
  await expect(gemini).toContainText('Learning default');

  const results = await new AxeBuilder({ page }).analyze();
  expect(results.violations).toEqual([]);
  await assertSyntheticKeysAreGone(page);
  expect([...external]).toEqual([]);
});

async function switchApplicationLanguageToEnglish(page: Page) {
  const trigger = page.locator('header').getByRole('button').first();
  if ((await trigger.textContent())?.trim() === 'English') return;
  await trigger.click();
  await page.getByRole('menuitemradio', { name: 'English' }).click();
  await expect(trigger).toHaveText('English');
}

test('three language entry points persist and onboarding passes axe', async ({
  page,
}) => {
  await page.goto('/onboarding');
  const language = page.getByRole('button', { name: '应用语言' });
  await language.click();
  await expect(
    page.getByRole('menuitemradio', { name: '简体中文' }),
  ).toBeVisible();
  await expect(
    page.getByRole('menuitemradio', { name: '繁體中文' }),
  ).toBeVisible();
  await expect(
    page.getByRole('menuitemradio', { name: 'English' }),
  ).toBeVisible();
  await page.getByRole('menuitemradio', { name: '繁體中文' }).click();
  await expect(page.getByRole('heading', { name: '開始使用' })).toBeVisible();

  await page.evaluate(() =>
    (
      window as Window & {
        __p7SeedReady(): void;
      }
    ).__p7SeedReady(),
  );
  await page.goto('/library');
  await page.getByRole('button', { name: '應用程式語言' }).click();
  await page.getByRole('menuitemradio', { name: 'English' }).click();
  await expect(
    page.getByRole('button', { name: 'Application language' }),
  ).toHaveText('English');

  await page.goto('/settings');
  await page.getByRole('button', { name: 'Application language' }).click();
  await page.getByRole('menuitemradio', { name: '简体中文' }).click();
  await expect(page.getByRole('button', { name: '应用语言' })).toHaveText(
    '简体中文',
  );

  await page.goto('/onboarding');
  const results = await new AxeBuilder({ page }).analyze();
  expect(results.violations).toEqual([]);
  await assertSyntheticKeysAreGone(page);
});
