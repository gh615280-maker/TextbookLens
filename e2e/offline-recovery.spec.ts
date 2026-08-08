import { expect, test, type BrowserContext, type Page } from '@playwright/test';

const BOOK_ID = '15000000-0000-4000-8000-000000000001';
const SECTION_ID = '15000000-0000-4000-8000-000000000002';
const BLOCK_ID = '15000000-0000-4000-8000-000000000003';
const PROFILE_ID = '15000000-0000-4000-8000-000000000004';
const CONVERSATION_ID = '15000000-0000-4000-8000-000000000005';
const TIME = '2026-08-07T00:00:00.000Z';
const PRIVATE_PATH = 'C:/private/P15_BACKUP_PATH.tlbackup';
const PRIVATE_KEY = 'P15_PRIVATE_KEY_MUST_NOT_RENDER';
const PRIVATE_PROMPT = 'P15_PRIVATE_PROMPT_MUST_NOT_RENDER';
const PRIVATE_BODY = 'P15_PROVIDER_BODY_MUST_NOT_RENDER';
const TEXTBOOK_TEXT =
  'Offline textbook body with an immutable local selection and durable notes.';

type Note = {
  id: string;
  bookId: string;
  sectionId: string;
  anchor: Record<string, unknown>;
  selectedText: string | null;
  noteText: string;
  revision: number;
  createdAt: string;
  updatedAt: string;
};

class OfflineBackend {
  readonly calls = new Map<string, number>();
  readonly blockedOrigins: string[] = [];
  readonly errors: string[] = [];
  notes: Note[] = [];
  networkAvailable = false;
  backupCount = 0;
  skipPromptConsent = true;
  nextNote = 1;

  invoke(command: string, payload: Record<string, unknown> = {}): unknown {
    this.calls.set(command, (this.calls.get(command) ?? 0) + 1);
    switch (command) {
      case 'get_app_settings':
      case 'initialize_ui_language':
        return this.settings();
      case 'update_ui_language':
        return this.settings();
      case 'get_onboarding_state':
        return {
          step: 'ready',
          selectedBook: this.book(),
          hasReadyBook: true,
          learningProfileConnected: true,
          visionProfileConnected: true,
          localTextQuality: 'ready',
          canSkipOnboarding: true,
        };
      case 'list_books':
        return [this.book()];
      case 'get_reader_bootstrap':
        return { book: this.book(), lastLocator: null };
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
      case 'complete_first_reader_hint':
        return this.settings();
      case 'list_reader_sections':
        return [this.section()];
      case 'read_derived_text':
        return `<p data-section-id="${SECTION_ID}" data-block-id="${BLOCK_ID}">${TEXTBOOK_TEXT}</p>`;
      case 'search_book':
        return [
          {
            snippet: 'offline searchable phrase',
            locator: this.locator(),
            sectionTitle: 'Offline section',
          },
        ];
      case 'save_reading_progress':
        return undefined;
      case 'list_provider_profiles':
        return [
          {
            id: PROFILE_ID,
            kind: 'openai',
            displayName: 'Synthetic offline profile',
            modelId: 'synthetic-model',
            contextWindowTokens: 32_000,
            isActive: true,
            credentialStatus: 'available',
            validatedAt: TIME,
          },
        ];
      case 'list_annotation_markers':
        return this.notes.map((note) => ({
          id: note.id,
          kind: 'note',
          conversationId: null,
          anchor: note.anchor,
          relocationStatus: 'primary',
          accessibilityLabel: 'View personal note',
        }));
      case 'create_note':
        return this.createNote(payload);
      case 'get_note':
        return this.getNote(payload);
      case 'update_note':
        return this.updateNote(payload);
      case 'delete_note':
        return this.deleteNote(payload);
      case 'list_notes':
        return this.notes.filter((note) => note.bookId === payload.bookId);
      case 'get_learning_overview':
        return this.overview();
      case 'list_book_learning_conversation_summaries':
        return [
          {
            id: CONVERSATION_ID,
            createdAt: TIME,
            updatedAt: TIME,
            messageCount: 2,
            firstQuestionPreview: 'Durable offline question',
          },
        ];
      case 'get_book_learning_conversation':
        return this.conversation();
      case 'get_maintenance_status':
        return { code: 'MAINTENANCE_AVAILABLE', activeOperations: [] };
      case 'get_storage_usage':
        return {
          totalBytes: 4096,
          totalFileCount: 3,
          categories: [
            { category: 'source', bytes: 2048, fileCount: 1 },
            { category: 'derived', bytes: 1024, fileCount: 1 },
            { category: 'database', bytes: 1024, fileCount: 1 },
          ],
        };
      case 'plugin:dialog|save':
        return PRIVATE_PATH;
      case 'create_local_backup':
        this.backupCount += 1;
        return { formatVersion: 1, archiveBytes: 4096, entryCount: 3 };
      case 'prepare_book_learning_request':
        if (!this.networkAvailable) throw { code: 'NETWORK_OFFLINE' };
        return {
          preparationId: '15000000-0000-4000-8000-000000000101',
          providerDisplayName: 'Synthetic provider',
          profileDisplayName: 'Synthetic offline profile',
          modelDisplayName: 'synthetic-model',
          estimatedInputTokens: 512,
          sourceCount: 2,
          citationCount: 1,
          omittedSourceCount: 0,
          riskFlags: [],
          requiresBlockingConfirmation: false,
          expiresAt: '2026-08-07T01:00:00.000Z',
        };
      case 'start_book_learning_request':
        return this.failedRequest();
      case 'subscribe_learning_request':
        return this.failedRequest();
      case 'prepare_learning_request':
      case 'confirm_index_operation':
      case 'create_index_run':
        throw { code: 'NETWORK_OFFLINE' };
      case 'discard_learning_preparation':
      case 'discard_book_learning_preparation':
      case 'cancel_learning_request':
        return undefined;
      default:
        throw new Error(`Unexpected IPC command: ${command}`);
    }
  }

  stats() {
    return {
      calls: Object.fromEntries(this.calls),
      notes: this.notes.map((note) => ({
        text: note.noteText,
        revision: note.revision,
      })),
      backupCount: this.backupCount,
      skipPromptConsent: this.skipPromptConsent,
    };
  }

  private settings() {
    return {
      onboardingCompleted: true,
      activeProviderProfileId: PROFILE_ID,
      defaultLearningProfileId: PROFILE_ID,
      defaultVisionProfileId: PROFILE_ID,
      theme: 'system',
      contextMode: 'standard',
      uiLanguage: 'en',
      uiLanguageInitialized: true,
      firstReaderHintCompleted: true,
    };
  }

  private book() {
    return {
      id: BOOK_ID,
      title: 'Offline recovery textbook',
      originalFilename: 'offline.docx',
      author: 'Synthetic author',
      language: 'en',
      format: 'docx',
      importStatus: 'ready',
      importErrorCode: null,
      importErrorMessage: null,
      importErrorStage: null,
      readingProgress: 0.25,
      fullTextQaReady: false,
      indexAggregate: {
        status: 'not_required',
        totalPages: 0,
        indexedPages: 0,
        reviewPages: 0,
        failedPages: 0,
      },
      createdAt: TIME,
      updatedAt: TIME,
      lastOpenedAt: TIME,
    };
  }

  private locator() {
    return {
      format: 'docx',
      startBlockId: BLOCK_ID,
      startOffset: 0,
      endBlockId: BLOCK_ID,
      endOffset: TEXTBOOK_TEXT.length,
    };
  }

  private section() {
    return {
      id: SECTION_ID,
      parentId: null,
      ordinal: 0,
      title: 'Offline section',
      locator: this.locator(),
    };
  }

  private createNote(payload: Record<string, unknown>): Note {
    const note: Note = {
      id: `15000000-0000-4000-8000-${String(200 + this.nextNote++).padStart(12, '0')}`,
      bookId: String(payload.bookId),
      sectionId: String(payload.sectionId),
      anchor: payload.anchor as Record<string, unknown>,
      selectedText: (payload.selectedText as string | null) ?? null,
      noteText: String(payload.noteText),
      revision: 1,
      createdAt: TIME,
      updatedAt: TIME,
    };
    this.notes.push(note);
    return structuredClone(note);
  }

  private getNote(payload: Record<string, unknown>): Note {
    const note = this.notes.find(
      (candidate) =>
        candidate.id === payload.noteId && candidate.bookId === payload.bookId,
    );
    if (!note) throw { code: 'NOT_FOUND' };
    return structuredClone(note);
  }

  private updateNote(payload: Record<string, unknown>): Note {
    const note = this.notes.find(
      (candidate) =>
        candidate.id === payload.noteId && candidate.bookId === payload.bookId,
    );
    if (!note || note.revision !== payload.expectedRevision)
      throw { code: 'REQUEST_CONFLICT' };
    note.noteText = String(payload.noteText);
    note.revision += 1;
    note.updatedAt = '2026-08-07T00:01:00.000Z';
    return structuredClone(note);
  }

  private deleteNote(payload: Record<string, unknown>) {
    const note = this.notes.find(
      (candidate) =>
        candidate.id === payload.noteId && candidate.bookId === payload.bookId,
    );
    if (!note || note.revision !== payload.expectedRevision)
      throw { code: 'REQUEST_CONFLICT' };
    this.notes = this.notes.filter((candidate) => candidate !== note);
    return undefined;
  }

  private overview() {
    return {
      bookId: BOOK_ID,
      format: 'docx',
      teachingInstructionConfigured: true,
      sectionCount: 1,
      sections: [
        {
          id: SECTION_ID,
          parentId: null,
          ordinal: 0,
          title: 'Offline section',
          localTextItemCount: 1,
          userNoteCount: this.notes.length,
          completedConversationCount: 1,
          completedExchangeCount: 1,
        },
      ],
      sources: [
        this.source('local_text', 1, true),
        this.source('ai_transcribed', 0, true),
        this.source('ai_description', 0, false),
        this.source('user_corrected', 0, true),
        this.source('user_note', this.notes.length, false),
        this.source('history_summary', 1, false),
      ],
      activity: {
        userNoteCount: this.notes.length,
        completedConversationCount: 1,
        completedExchangeCount: 1,
        citationCount: 1,
      },
    };
  }

  private source(source: string, itemCount: number, quoteable: boolean) {
    return {
      source,
      itemCount,
      coveredSectionCount: itemCount > 0 ? 1 : 0,
      coveredPageCount: 0,
      quoteableAsTextbook: quoteable,
    };
  }

  private conversation() {
    return {
      id: CONVERSATION_ID,
      bookId: BOOK_ID,
      scope: 'book',
      status: 'completed',
      messages: [
        {
          id: '15000000-0000-4000-8000-000000000011',
          ordinal: 0,
          role: 'user',
          action: 'ask',
          content: 'Durable offline question',
          providerId: null,
          modelId: null,
          citations: [],
          createdAt: TIME,
        },
        {
          id: '15000000-0000-4000-8000-000000000012',
          ordinal: 1,
          role: 'assistant',
          action: 'ask',
          content: 'Durable offline answer',
          providerId: PROFILE_ID,
          modelId: 'synthetic-model',
          citations: [
            {
              id: 'TL-C1',
              label: 'Offline local citation',
              bookId: BOOK_ID,
              sectionId: SECTION_ID,
              locator: this.locator(),
              source: 'local_text',
              reviewStatus: 'not_required',
              quoteable: true,
            },
          ],
          createdAt: TIME,
        },
      ],
      createdAt: TIME,
      updatedAt: TIME,
    };
  }

  private failedRequest() {
    return {
      requestId: '15000000-0000-4000-8000-000000000102',
      conversationId: null,
      status: 'failed',
      text: '',
      usage: null,
      safeError: { code: 'PROVIDER_UNAVAILABLE' },
      lastSeq: 1,
    };
  }
}

test('offline library, reader, search, notes, history, overview, and backup survive restart', async ({
  page,
  context,
}) => {
  const backend = new OfflineBackend();
  await installStrictBackend(page, context, backend);
  await page.goto('/library');
  await context.setOffline(true);

  await expect(
    page.getByRole('button', {
      name: 'Offline recovery textbook',
      exact: true,
    }),
  ).toBeVisible();
  await navigateSpa(page, `/books/${BOOK_ID}/read`);
  await expect(page.getByText(TEXTBOOK_TEXT)).toBeVisible();

  await page.getByRole('button', { name: 'Search' }).click();
  const search = page.getByRole('dialog', { name: 'Search this book' });
  await search.getByRole('textbox').fill('offline');
  await search.getByRole('button', { name: 'Search' }).click();
  await expect(search.getByText('offline searchable phrase')).toBeVisible();
  await page.keyboard.press('Escape');

  await selectText(page);
  await page.getByRole('menuitem', { name: 'Add note' }).click();
  await page
    .getByRole('textbox', { name: 'Personal note' })
    .fill('durable offline note');
  await page.getByRole('button', { name: 'Save note' }).click();
  const marker = page
    .locator('[data-annotation-id] [data-marker-shape="note"]')
    .first();
  await expect(marker).toBeVisible();
  await marker.click();
  const editor = page.getByRole('textbox', { name: 'Personal note' });
  await editor.fill('edited durable offline note');
  await page.getByRole('button', { name: 'Save note' }).click();

  // The web harness needs loopback briefly to reload its bundled assets. All
  // external origins remain intercepted, while Tauri IPC stays mocked.
  await context.setOffline(false);
  await page.reload();
  await context.setOffline(true);
  await expect(page.getByText(TEXTBOOK_TEXT)).toBeVisible();
  const restartedMarker = page
    .locator('[data-annotation-id] [data-marker-shape="note"]')
    .first();
  await expect(restartedMarker).toBeVisible();
  await restartedMarker.click();
  await expect(
    page.getByRole('textbox', { name: 'Personal note' }),
  ).toHaveValue('edited durable offline note');

  await navigateSpa(page, `/books/${BOOK_ID}/overview`);
  await expect(
    page.getByRole('heading', { name: 'Learning overview' }),
  ).toBeVisible();
  await page.getByText('Local learning overview', { exact: true }).click();
  await expect(
    page.getByText('Offline section', { exact: false }).first(),
  ).toBeVisible();
  await page.getByText('Book conversations', { exact: true }).click();
  await page.getByRole('button', { name: 'Durable offline question' }).click();
  await expect(page.getByText('Durable offline answer')).toBeVisible();

  await navigateSpa(page, '/settings');
  await page.getByRole('button', { name: 'Create backup' }).click();
  await page
    .getByRole('dialog', { name: 'Create local backup' })
    .getByRole('button', { name: 'Choose .tlbackup location' })
    .click();
  await expect(
    page.getByRole('status', { name: 'Settings operation status' }),
  ).toContainText('Backup created safely.');

  const stats = backend.stats();
  expect(stats.notes).toEqual([
    { text: 'edited durable offline note', revision: 2 },
  ]);
  expect(stats.backupCount).toBe(1);
  expect(providerCalls(stats.calls)).toEqual({});
  await assertPrivateAndLocal(page, backend);
});

test('offline AI controls fail safely and network return never resumes without a new user action', async ({
  page,
  context,
}) => {
  const backend = new OfflineBackend();
  await installStrictBackend(page, context, backend);
  await page.goto(`/books/${BOOK_ID}/overview`);
  await context.setOffline(true);

  expect(providerCalls(backend.stats().calls)).toEqual({});
  await page.getByLabel('Question').fill('Offline question');
  await page.getByRole('button', { name: 'Send' }).click();
  await expect(page.getByRole('alert')).toContainText(
    'The request could not be prepared or started safely.',
  );
  await expect(
    page.getByRole('alert').getByRole('link', { name: 'AI services' }),
  ).toBeVisible();
  expect(backend.calls.get('prepare_book_learning_request')).toBe(1);
  expect(backend.calls.get('start_book_learning_request') ?? 0).toBe(0);

  await navigateSpa(page, `/books/${BOOK_ID}/read`);
  await expect(page.getByText(TEXTBOOK_TEXT)).toBeVisible();
  await selectText(page);
  await page.getByRole('menuitem', { name: 'Explain' }).click();
  await expect(
    page.getByRole('status').filter({
      hasText: 'This learning action could not be prepared locally.',
    }),
  ).toBeVisible();
  expect(backend.calls.get('prepare_learning_request')).toBe(1);
  expect(backend.calls.get('start_learning_request') ?? 0).toBe(0);

  await context.setOffline(false);
  backend.networkAvailable = true;
  await page.reload();
  await navigateSpa(page, '/library');
  await navigateSpa(page, `/books/${BOOK_ID}/overview`);
  await expect(
    page.getByRole('heading', { name: 'Learning overview' }),
  ).toBeVisible();
  expect(backend.calls.get('prepare_book_learning_request')).toBe(1);
  expect(backend.calls.get('start_book_learning_request') ?? 0).toBe(0);
  expect(backend.calls.get('confirm_index_operation') ?? 0).toBe(0);
  expect(backend.calls.get('create_index_run') ?? 0).toBe(0);
  expect(backend.skipPromptConsent).toBe(true);

  await page.getByLabel('Question').fill('Explicit retry after network return');
  await page.getByRole('button', { name: 'Send' }).click();
  await expect(page.getByRole('heading', { name: 'Failed' })).toBeVisible();
  expect(backend.calls.get('prepare_book_learning_request')).toBe(2);
  expect(backend.calls.get('start_book_learning_request')).toBe(1);
  expect(backend.calls.get('subscribe_learning_request')).toBe(1);
  expect(backend.calls.get('authorize_book_learning_request') ?? 0).toBe(0);
  await assertPrivateAndLocal(page, backend);
});

async function installStrictBackend(
  page: Page,
  context: BrowserContext,
  backend: OfflineBackend,
) {
  await context.route(/^(?:https?):\/\//u, async (route) => {
    const url = new URL(route.request().url());
    if (['127.0.0.1', 'localhost'].includes(url.hostname)) {
      await route.continue();
      return;
    }
    backend.blockedOrigins.push(url.origin);
    await route.abort('blockedbyclient');
  });
  page.on('pageerror', (error) => backend.errors.push(error.message));
  page.on('console', (message) => {
    if (message.type() === 'error') backend.errors.push(message.text());
  });
  await page.exposeFunction(
    '__phase15Invoke',
    (command: string, payload?: Record<string, unknown>) =>
      backend.invoke(command, payload),
  );
  await page.addInitScript(() => {
    type TestWindow = Window & {
      __phase15Invoke(
        command: string,
        payload?: Record<string, unknown>,
      ): Promise<unknown>;
      __TAURI_INTERNALS__: {
        transformCallback(callback: (payload: unknown) => void): number;
        unregisterCallback(id: number): void;
        invoke(
          command: string,
          payload?: Record<string, unknown>,
        ): Promise<unknown>;
      };
    };
    const target = window as TestWindow;
    let callbackId = 1;
    target.__TAURI_INTERNALS__ = {
      transformCallback() {
        return callbackId++;
      },
      unregisterCallback() {},
      async invoke(command, payload = {}) {
        const safePayload =
          command === 'subscribe_learning_request'
            ? {
                requestId: payload.requestId,
                afterSeq: payload.afterSeq,
              }
            : payload;
        return target.__phase15Invoke(command, safePayload);
      },
    };
  });
}

async function navigateSpa(page: Page, path: string) {
  await page.evaluate((nextPath) => {
    history.pushState({}, '', nextPath);
    dispatchEvent(new PopStateEvent('popstate'));
  }, path);
}

async function selectText(page: Page) {
  await page.evaluate((blockId) => {
    const block = document.querySelector(`[data-block-id="${blockId}"]`);
    const node = block?.firstChild;
    if (!(node instanceof Text)) throw new Error('missing local text node');
    const range = document.createRange();
    range.setStart(node, 0);
    range.setEnd(node, Math.min(node.length, 45));
    const selection = getSelection();
    selection?.removeAllRanges();
    selection?.addRange(range);
    block.dispatchEvent(new MouseEvent('mouseup', { bubbles: true }));
  }, BLOCK_ID);
  await expect(
    page.getByRole('menu', { name: 'Learning actions' }),
  ).toBeVisible();
}

function providerCalls(calls: Record<string, number>) {
  return Object.fromEntries(
    Object.entries(calls).filter(([command]) =>
      [
        'prepare_learning_request',
        'start_learning_request',
        'prepare_book_learning_request',
        'start_book_learning_request',
        'confirm_index_operation',
        'create_index_run',
      ].includes(command),
    ),
  );
}

async function assertPrivateAndLocal(page: Page, backend: OfflineBackend) {
  const body = await page.locator('body').innerText();
  for (const privateValue of [
    PRIVATE_PATH,
    PRIVATE_KEY,
    PRIVATE_PROMPT,
    PRIVATE_BODY,
  ]) {
    expect(body).not.toContain(privateValue);
    expect(JSON.stringify(backend.stats())).not.toContain(privateValue);
  }
  expect(backend.blockedOrigins).toEqual([]);
  expect(backend.errors).toEqual([]);
}
