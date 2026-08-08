import AxeBuilder from '@axe-core/playwright';
import { expect, test, type Page } from '@playwright/test';

const BOOK_A = '14000000-0000-4000-8000-000000000001';
const BOOK_B = '14000000-0000-4000-8000-000000000002';
const SECTION_A = '14000000-0000-4000-8000-000000000011';
const SECTION_B = '14000000-0000-4000-8000-000000000012';
const OLD_PROFILE = '14000000-0000-4000-8000-000000000021';
const NEW_PROFILE = '14000000-0000-4000-8000-000000000022';
const TIME = '2026-08-06T00:00:00.000Z';
const PRIVATE_REASONING = 'P14_BROWSER_PRIVATE_REASONING_SENTINEL';
const PRIVATE_PATH = 'C:/private/P14_BROWSER_PATH_SENTINEL.pdf';
const PRIVATE_KEY = 'P14_BROWSER_KEY_SENTINEL';
const PRIVATE_IMAGE = 'P14_BROWSER_IMAGE_SENTINEL';
const PRIVATE_PROMPT = 'P14_BROWSER_PROMPT_SENTINEL';
const SAFE_ANSWER = [
  'Visible **bounded answer**.',
  '<script>P14_RAW_HTML_SENTINEL</script>',
  '[unsafe](javascript:alert(1))',
  '![remote image](https://never-request.invalid/private.png)',
  `<reasoning>${PRIVATE_REASONING}</reasoning>`,
  '[safe reference](https://example.invalid/reference)',
].join('\n\n');

type RequestStatus =
  'preparing' | 'streaming' | 'completed' | 'failed' | 'cancelled';

interface Preparation {
  id: string;
  bookId: string;
  conversationId: string | null;
  question: string;
  profileId: string;
  modelId: string;
  blocking: boolean;
  authorized: boolean;
}

interface RequestState {
  requestId: string;
  preparationId: string;
  bookId: string;
  targetConversationId: string | null;
  question: string;
  profileId: string;
  modelId: string;
  status: RequestStatus;
  text: string;
  lastSeq: number;
  conversationId: string | null;
}

interface Message {
  id: string;
  ordinal: number;
  role: 'user' | 'assistant';
  action: 'ask' | 'continue';
  content: string;
  providerId: string | null;
  modelId: string | null;
  citations: Array<Record<string, unknown>>;
  createdAt: string;
}

interface Conversation {
  id: string;
  bookId: string;
  scope: 'book';
  status: 'completed';
  messages: Message[];
  createdAt: string;
  updatedAt: string;
}

interface TransportEnvelope {
  __p14Response: unknown;
  __p14Events: Array<Record<string, unknown>>;
}

class SyntheticBookBackend {
  readonly calls = new Map<string, number>();
  readonly preparations = new Map<string, Preparation>();
  readonly requests = new Map<string, RequestState>();
  readonly conversations = new Map<string, Conversation>();
  readonly externalOrigins = new Set<string>();
  readonly consoleErrors: string[] = [];
  selectionRequestActive = true;
  providerAvailable = false;
  currentProfile = { id: OLD_PROFILE, modelId: 'synthetic-old-model' };
  cancellationCalls: string[] = [];
  authorizationCalls = 0;
  successfulTransactions = 0;
  restartCount = 0;
  failNextDelete = false;
  returnDeletedSummary = false;
  private deletedSummary: Record<string, unknown> | null = null;
  private nextId = 100;
  private language: 'en' | 'zh-CN' | 'zh-TW' = 'en';

  invoke(command: string, payload: Record<string, unknown> = {}): unknown {
    this.calls.set(command, (this.calls.get(command) ?? 0) + 1);
    switch (command) {
      case 'get_app_settings':
      case 'initialize_ui_language':
        return this.settings();
      case 'update_ui_language':
        this.language = String(payload.language) as typeof this.language;
        return this.settings();
      case 'get_learning_overview':
        return this.overview(String(payload.bookId));
      case 'list_book_learning_conversation_summaries':
        return this.list(String(payload.bookId));
      case 'get_book_learning_conversation':
        return this.getConversation(
          String(payload.bookId),
          String(payload.conversationId),
        );
      case 'delete_book_learning_conversation':
        return this.deleteConversation(
          String(payload.bookId),
          String(payload.conversationId),
        );
      case 'prepare_book_learning_request':
        return this.prepare(payload.metadata as Record<string, unknown>);
      case 'authorize_book_learning_request':
        return this.authorize(String(payload.preparationId));
      case 'discard_book_learning_preparation':
        this.preparations.delete(String(payload.preparationId));
        return undefined;
      case 'start_book_learning_request':
        return this.start(String(payload.preparationId));
      case 'subscribe_learning_request':
        return this.subscribe(
          String(payload.requestId),
          Number(payload.afterSeq),
        );
      case 'cancel_learning_request':
        return this.cancel(String(payload.requestId));
      default:
        throw new Error(`Unexpected IPC command: ${command}`);
    }
  }

  advance(
    requestId: string,
    step:
      | { type: 'text_delta'; text: string }
      | { type: 'usage'; inputTokens: number; outputTokens: number }
      | { type: 'completed' },
  ): Record<string, unknown> {
    const request = this.request(requestId);
    if (request.status === 'completed' || request.status === 'cancelled') {
      throw new Error('late synthetic event rejected');
    }
    request.lastSeq += 1;
    if (step.type === 'text_delta') {
      request.status = 'streaming';
      request.text += step.text;
      return {
        requestId,
        seq: request.lastSeq,
        event: { type: 'text_delta', text: step.text },
      };
    }
    if (step.type === 'usage') {
      return {
        requestId,
        seq: request.lastSeq,
        event: {
          type: 'usage',
          inputTokens: step.inputTokens,
          outputTokens: step.outputTokens,
        },
      };
    }
    if (!request.text.trim()) throw new Error('empty completion');
    const conversationId = this.commit(request);
    request.status = 'completed';
    request.conversationId = conversationId;
    return {
      requestId,
      seq: request.lastSeq,
      event: { type: 'completed', conversationId },
    };
  }

  seedConversation(bookId: string, question: string, answer: string): string {
    const request: RequestState = {
      requestId: this.uuid(),
      preparationId: this.uuid(),
      bookId,
      targetConversationId: null,
      question,
      profileId: OLD_PROFILE,
      modelId: 'synthetic-old-model',
      status: 'streaming',
      text: answer,
      lastSeq: 1,
      conversationId: null,
    };
    return this.commit(request);
  }

  switchProvider(): void {
    this.currentProfile = {
      id: NEW_PROFILE,
      modelId: 'synthetic-current-model',
    };
  }

  restart(): void {
    this.restartCount += 1;
    this.preparations.clear();
    this.requests.clear();
  }

  stats() {
    return {
      calls: Object.fromEntries(this.calls),
      cancellations: [...this.cancellationCalls],
      authorizations: this.authorizationCalls,
      transactions: this.successfulTransactions,
      conversations: this.conversations.size,
      selectionRequestActive: this.selectionRequestActive,
      restartCount: this.restartCount,
    };
  }

  private settings() {
    return {
      onboardingCompleted: true,
      activeProviderProfileId: this.currentProfile.id,
      defaultLearningProfileId: this.currentProfile.id,
      defaultVisionProfileId: null,
      theme: 'system',
      contextMode: 'standard',
      uiLanguage: this.language,
      uiLanguageInitialized: true,
      firstReaderHintCompleted: true,
    };
  }

  private overview(bookId: string) {
    if (bookId !== BOOK_A && bookId !== BOOK_B) throw { code: 'NOT_FOUND' };
    const sectionId = bookId === BOOK_A ? SECTION_A : SECTION_B;
    return {
      bookId,
      format: bookId === BOOK_A ? 'pdf' : 'epub',
      teachingInstructionConfigured: true,
      sectionCount: 1,
      sections: [
        {
          id: sectionId,
          parentId: null,
          ordinal: 0,
          title: bookId === BOOK_A ? 'Synthetic alpha' : 'Synthetic beta',
          localTextItemCount: 2,
          userNoteCount: 1,
          completedConversationCount: 0,
          completedExchangeCount: 0,
        },
      ],
      sources: [
        this.source('local_text', 2, 1, 0, true),
        this.source('ai_transcribed', 1, 0, 1, true),
        this.source('ai_description', 1, 0, 1, false),
        this.source('user_corrected', 1, 0, 1, true),
        this.source('user_note', 1, 1, 0, false),
        this.source('history_summary', 0, 0, 0, false),
      ],
      activity: {
        userNoteCount: 1,
        completedConversationCount: 0,
        completedExchangeCount: 0,
        citationCount: 0,
      },
    };
  }

  private source(
    source: string,
    itemCount: number,
    coveredSectionCount: number,
    coveredPageCount: number,
    quoteableAsTextbook: boolean,
  ) {
    return {
      source,
      itemCount,
      coveredSectionCount,
      coveredPageCount,
      quoteableAsTextbook,
    };
  }

  private prepare(metadata: Record<string, unknown>) {
    const bookId = String(metadata.bookId);
    const conversationId =
      metadata.kind === 'continue' ? String(metadata.conversationId) : null;
    if (bookId !== BOOK_A && bookId !== BOOK_B) throw { code: 'NOT_FOUND' };
    if (
      conversationId &&
      this.conversations.get(conversationId)?.bookId !== bookId
    ) {
      throw { code: 'REQUEST_CONFLICT' };
    }
    const id = this.uuid();
    const blocking = conversationId === null;
    this.preparations.set(id, {
      id,
      bookId,
      conversationId,
      question: String(metadata.question),
      profileId: this.currentProfile.id,
      modelId: this.currentProfile.modelId,
      blocking,
      authorized: !blocking,
    });
    return {
      preparationId: id,
      providerDisplayName: 'Synthetic provider',
      profileDisplayName: 'Synthetic profile',
      modelDisplayName: this.currentProfile.modelId,
      estimatedInputTokens: blocking ? 40_000 : 512,
      sourceCount: 3,
      citationCount: 1,
      omittedSourceCount: 0,
      riskFlags: blocking ? ['cost_risk'] : [],
      requiresBlockingConfirmation: blocking,
      expiresAt: TIME,
    };
  }

  private authorize(preparationId: string) {
    const value = this.preparations.get(preparationId);
    if (!value) throw { code: 'NOT_FOUND' };
    value.authorized = true;
    this.authorizationCalls += 1;
    return undefined;
  }

  private start(preparationId: string) {
    const preparation = this.preparations.get(preparationId);
    if (!preparation || (preparation.blocking && !preparation.authorized)) {
      throw { code: 'REQUEST_CONFLICT' };
    }
    this.preparations.delete(preparationId);
    const requestId = this.uuid();
    const request: RequestState = {
      requestId,
      preparationId,
      bookId: preparation.bookId,
      targetConversationId: preparation.conversationId,
      question: preparation.question,
      profileId: preparation.profileId,
      modelId: preparation.modelId,
      status: 'preparing',
      text: '',
      lastSeq: 0,
      conversationId: null,
    };
    this.requests.set(requestId, request);
    return this.snapshot(request);
  }

  private subscribe(requestId: string, afterSeq: number): TransportEnvelope {
    const request = this.request(requestId);
    if (afterSeq > request.lastSeq) throw { code: 'INVALID_INPUT' };
    return { __p14Response: this.snapshot(request), __p14Events: [] };
  }

  private cancel(requestId: string): TransportEnvelope {
    const request = this.request(requestId);
    if (request.status === 'completed' || request.status === 'cancelled') {
      return { __p14Response: undefined, __p14Events: [] };
    }
    request.status = 'cancelled';
    request.lastSeq += 1;
    this.cancellationCalls.push(requestId);
    return {
      __p14Response: undefined,
      __p14Events: [
        {
          requestId,
          seq: request.lastSeq,
          event: { type: 'cancelled' },
        },
      ],
    };
  }

  private commit(request: RequestState): string {
    const existing = request.targetConversationId
      ? this.conversations.get(request.targetConversationId)
      : null;
    if (request.targetConversationId && !existing) {
      throw new Error('deleted conversation cannot revive');
    }
    if (existing) {
      const ordinal = existing.messages.length;
      existing.messages.push(
        this.userMessage(ordinal, 'continue', request.question),
        this.assistantMessage(
          ordinal + 1,
          'continue',
          request.text,
          request.profileId,
          request.modelId,
          request.bookId,
        ),
      );
      existing.updatedAt = TIME;
      this.successfulTransactions += 1;
      return existing.id;
    }
    const id = this.uuid();
    this.conversations.set(id, {
      id,
      bookId: request.bookId,
      scope: 'book',
      status: 'completed',
      messages: [
        this.userMessage(0, 'ask', request.question),
        this.assistantMessage(
          1,
          'ask',
          request.text,
          request.profileId,
          request.modelId,
          request.bookId,
        ),
      ],
      createdAt: TIME,
      updatedAt: TIME,
    });
    this.successfulTransactions += 1;
    return id;
  }

  private userMessage(
    ordinal: number,
    action: 'ask' | 'continue',
    content: string,
  ): Message {
    return {
      id: this.uuid(),
      ordinal,
      role: 'user',
      action,
      content,
      providerId: null,
      modelId: null,
      citations: [],
      createdAt: TIME,
    };
  }

  private assistantMessage(
    ordinal: number,
    action: 'ask' | 'continue',
    content: string,
    providerId: string,
    modelId: string,
    bookId: string,
  ): Message {
    return {
      id: this.uuid(),
      ordinal,
      role: 'assistant',
      action,
      content,
      providerId,
      modelId,
      citations: [
        {
          id: 'TL-C1',
          label: 'Synthetic local citation',
          bookId,
          sectionId: bookId === BOOK_A ? SECTION_A : SECTION_B,
          locator: {
            format: 'pdf',
            startPage: 1,
            endPage: 1,
            rectsByPage: null,
          },
          source: 'local_text',
          reviewStatus: 'not_required',
          quoteable: true,
        },
      ],
      createdAt: TIME,
    };
  }

  private list(bookId: string) {
    const values = [...this.conversations.values()]
      .filter((value) => value.bookId === bookId)
      .map((value) => this.summary(value));
    if (this.returnDeletedSummary && this.deletedSummary) {
      values.push(this.deletedSummary);
    }
    return values;
  }

  private summary(value: Conversation) {
    return {
      id: value.id,
      createdAt: value.createdAt,
      updatedAt: value.updatedAt,
      messageCount: value.messages.length,
      firstQuestionPreview: value.messages[0]!.content.slice(0, 280),
    };
  }

  private getConversation(bookId: string, conversationId: string) {
    const value = this.conversations.get(conversationId);
    if (!value || value.bookId !== bookId) throw { code: 'NOT_FOUND' };
    return structuredClone(value);
  }

  private deleteConversation(bookId: string, conversationId: string) {
    const value = this.conversations.get(conversationId);
    if (!value || value.bookId !== bookId) throw { code: 'NOT_FOUND' };
    if (
      [...this.requests.values()].some(
        (request) =>
          request.targetConversationId === conversationId &&
          !['completed', 'cancelled', 'failed'].includes(request.status),
      )
    ) {
      throw { code: 'REQUEST_CONFLICT' };
    }
    if (this.failNextDelete) {
      this.failNextDelete = false;
      throw { code: 'DATABASE_ERROR' };
    }
    this.deletedSummary = this.summary(value);
    this.conversations.delete(conversationId);
    return undefined;
  }

  private snapshot(request: RequestState) {
    return {
      requestId: request.requestId,
      conversationId: request.conversationId,
      status: request.status,
      text: request.text,
      usage: null,
      safeError: null,
      lastSeq: request.lastSeq,
    };
  }

  private request(requestId: string): RequestState {
    const request = this.requests.get(requestId);
    if (!request) throw new Error('missing synthetic request');
    return request;
  }

  private uuid(): string {
    const suffix = String(this.nextId++).padStart(12, '0');
    return `14000000-0000-4000-8000-${suffix}`;
  }
}

test('local overview stays offline, source-explicit, trilingual, accessible, and responsive', async ({
  page,
  context,
}) => {
  const backend = new SyntheticBookBackend();
  await installBackend(page, backend);
  await page.goto(`/books/${BOOK_A}/overview`);

  await expect(
    page.getByRole('heading', { name: 'Learning overview' }),
  ).toBeVisible();
  await page.getByText('Local learning overview', { exact: true }).click();
  await expect(page.getByText('This local overview never sends')).toBeVisible();
  for (const source of [
    'local_text',
    'ai_transcribed',
    'ai_description',
    'user_corrected',
    'user_note',
    'history_summary',
  ]) {
    await expect(page.getByText(source, { exact: true })).toBeVisible();
  }
  const coverage = page.getByRole('region', { name: 'Source coverage' });
  await expect(coverage).toContainText(
    'local_text: 2 items; 1 sections; 0 pages; May be quoted',
  );
  await expect(coverage).toContainText(
    'ai_transcribed: 1 items; 0 sections; 1 pages; May be quoted',
  );
  await expect(coverage).toContainText(
    'ai_description: 1 items; 0 sections; 1 pages; Not textbook text',
  );
  await expect(coverage).toContainText(
    'user_corrected: 1 items; 0 sections; 1 pages; May be quoted',
  );
  await expect(coverage).toContainText(
    'user_note: 1 items; 1 sections; 0 pages; Not textbook text',
  );
  await expect(coverage).toContainText(
    'history_summary: 0 items; 0 sections; 0 pages; Not textbook text',
  );
  expect(backend.calls.get('prepare_book_learning_request') ?? 0).toBe(0);

  await page.getByRole('button', { name: 'Application language' }).click();
  await page.getByRole('menuitemradio', { name: '简体中文' }).click();
  await expect(page.getByRole('heading', { name: '学习总览' })).toBeVisible();
  await page.getByRole('button', { name: '应用语言' }).click();
  await page.getByRole('menuitemradio', { name: '繁體中文' }).click();
  await expect(page.getByRole('heading', { name: '學習總覽' })).toBeVisible();
  await page.getByRole('button', { name: '應用程式語言' }).click();
  await page.getByRole('menuitemradio', { name: 'English' }).click();
  await expect(
    page.getByRole('heading', { name: 'Learning overview' }),
  ).toBeVisible();

  await context.setOffline(true);
  await navigateSpa(page, `/books/${BOOK_B}/overview`);
  await page.getByText('Local learning overview', { exact: true }).click();
  await expect(page.getByText('Synthetic beta')).toBeVisible();
  await context.setOffline(false);
  expect(backend.providerAvailable).toBe(false);

  await page.setViewportSize({ width: 360, height: 720 });
  await expect
    .poll(() =>
      page.evaluate(() => document.body.scrollWidth <= innerWidth + 1),
    )
    .toBe(true);
  await page.evaluate(() => {
    document.documentElement.style.fontSize = '200%';
  });
  await expect(
    page.getByRole('heading', { name: 'Learning overview' }),
  ).toBeVisible();
  await page.emulateMedia({ forcedColors: 'active', reducedMotion: 'reduce' });
  await assertAccessibleAndPrivate(page, backend);
});

test('book requests authorize, stream independently, reconnect across routes, and stop explicitly', async ({
  page,
}) => {
  const backend = new SyntheticBookBackend();
  await installBackend(page, backend);
  await page.goto(`/books/${BOOK_A}/overview`);

  const send = page.getByRole('button', { name: 'Send' });
  await page.getByLabel('Question').fill('Alpha question');
  await send.click();
  const confirmation = page.getByRole('dialog', {
    name: 'Confirm learning request',
  });
  await expect(confirmation).toBeVisible();
  await expect(
    confirmation.getByRole('button', { name: 'Authorize and continue' }),
  ).toBeFocused();
  await page.keyboard.press('Escape');
  await expect(confirmation).toHaveCount(0);
  await expect(send).toBeFocused();

  await page.getByLabel('Question').fill('Alpha question');
  await send.click();
  await page
    .getByRole('dialog', { name: 'Confirm learning request' })
    .getByRole('button', { name: 'Authorize and continue' })
    .click();
  const alphaRequest = lastRequest(backend);
  await waitForChannel(page, alphaRequest);
  await emit(page, backend, alphaRequest, {
    type: 'text_delta',
    text: 'Alpha streaming answer',
  });
  await expect(page.getByText('Alpha streaming answer')).toBeVisible();

  await navigateSpa(page, `/books/${BOOK_B}/overview`);
  expect(backend.cancellationCalls).toEqual([]);
  await navigateSpa(page, `/books/${BOOK_A}/overview`);
  await expect(page.getByText('Alpha streaming answer')).toBeVisible();
  await navigateSpa(page, `/books/${BOOK_B}/overview`);
  await page.getByLabel('Question').fill('Beta question');
  await page.getByRole('button', { name: 'Send' }).click();
  await page
    .getByRole('dialog', { name: 'Confirm learning request' })
    .getByRole('button', { name: 'Authorize and continue' })
    .click();
  const betaRequest = lastRequest(backend);
  await waitForChannel(page, betaRequest);
  await emit(page, backend, betaRequest, {
    type: 'text_delta',
    text: 'Beta independent answer',
  });

  await navigateSpa(page, `/books/${BOOK_A}/overview`);
  await page.getByRole('button', { name: 'Stop' }).click();
  await expect(page.getByRole('heading', { name: 'Cancelled' })).toBeVisible();
  expect(backend.requests.get(alphaRequest)?.status).toBe('cancelled');
  expect(backend.requests.get(betaRequest)?.status).toBe('streaming');
  expect(backend.selectionRequestActive).toBe(true);

  await emit(page, backend, betaRequest, { type: 'completed' });
  await navigateSpa(page, `/books/${BOOK_B}/overview`);
  await page.getByText('Book conversations', { exact: true }).click();
  await expect(
    page.getByRole('button', { name: 'Beta question' }),
  ).toBeVisible();
  expect(backend.successfulTransactions).toBe(1);
  expect(backend.authorizationCalls).toBe(2);
  expect(backend.cancellationCalls).toEqual([alphaRequest]);
  await expect(page.locator('[data-panel-id]')).toHaveCount(0);
  await expect(page.getByRole('button', { name: /marker/iu })).toHaveCount(0);
  await assertAccessibleAndPrivate(page, backend);
});

test('history is lazy and durable; follow-up captures current provider; delete rolls back then tombstones', async ({
  page,
}) => {
  const backend = new SyntheticBookBackend();
  const conversationId = backend.seedConversation(
    BOOK_A,
    'Durable safe question',
    SAFE_ANSWER,
  );
  await installBackend(page, backend);
  await page.goto(`/books/${BOOK_A}/overview`);
  await page.getByText('Book conversations', { exact: true }).click();

  await expect(
    page.getByRole('button', { name: 'Durable safe question' }),
  ).toBeVisible();
  expect(backend.calls.get('get_book_learning_conversation') ?? 0).toBe(0);
  await page.getByRole('button', { name: 'Durable safe question' }).click();
  expect(backend.calls.get('get_book_learning_conversation')).toBe(1);
  await expect(page.getByText('Visible bounded answer.')).toBeVisible();
  await expect(page.getByText('unsafe (link blocked)')).toBeVisible();
  await expect(
    page.getByRole('link', { name: 'safe reference' }),
  ).toHaveAttribute('rel', /noopener/u);
  await expect(page.getByText('Synthetic local citation')).toBeVisible();
  await expect(page.locator('img')).toHaveCount(0);
  await expect(
    page.locator('[data-testid="safe-answer-renderer"] script'),
  ).toHaveCount(0);
  await expect(page.locator('body')).not.toContainText(PRIVATE_REASONING);
  await expect(
    page.getByText('<script>P14_RAW_HTML_SENTINEL</script>'),
  ).toBeVisible();

  backend.switchProvider();
  await page.getByLabel('Question').fill('Current provider follow-up');
  await page.getByRole('button', { name: 'Follow up' }).click();
  const followupRequest = lastRequest(backend);
  await waitForChannel(page, followupRequest);
  expect(backend.requests.get(followupRequest)?.targetConversationId).toBe(
    conversationId,
  );
  expect(backend.requests.get(followupRequest)?.profileId).toBe(NEW_PROFILE);
  expect(backend.requests.get(followupRequest)?.modelId).toBe(
    'synthetic-current-model',
  );

  await page.getByRole('button', { name: 'Delete' }).first().click();
  const activeDelete = page.getByRole('dialog', {
    name: 'Delete textbook question history',
  });
  await expect(
    activeDelete.getByRole('button', { name: 'Delete' }),
  ).toBeDisabled();
  await expect(activeDelete).toContainText('Stop the active request');
  await page.keyboard.press('Escape');
  await expect(activeDelete).toHaveCount(0);
  await expect(
    page.getByRole('button', { name: 'Delete' }).first(),
  ).toBeFocused();

  await emit(page, backend, followupRequest, {
    type: 'text_delta',
    text: 'Current provider durable follow-up',
  });
  await emit(page, backend, followupRequest, { type: 'completed' });
  const persisted = backend.conversations.get(conversationId)!;
  expect(persisted.messages[1]!.providerId).toBe(OLD_PROFILE);
  expect(persisted.messages[1]!.modelId).toBe('synthetic-old-model');
  expect(persisted.messages[3]!.providerId).toBe(NEW_PROFILE);
  expect(persisted.messages[3]!.modelId).toBe('synthetic-current-model');

  backend.restart();
  await page.reload();
  await page.getByText('Book conversations', { exact: true }).click();
  await expect(
    page.getByRole('button', { name: 'Durable safe question' }),
  ).toBeVisible();
  await page.getByRole('button', { name: 'Durable safe question' }).click();
  await expect(
    page.getByText('Current provider durable follow-up'),
  ).toBeVisible();
  await expect(page.getByText(/synthetic-old-model/u)).toBeVisible();
  await expect(page.getByText(/synthetic-current-model/u)).toBeVisible();

  backend.failNextDelete = true;
  const deleteTrigger = page.getByRole('button', { name: 'Delete' }).first();
  await deleteTrigger.click();
  const dialog = page.getByRole('dialog', {
    name: 'Delete textbook question history',
  });
  await dialog.getByRole('button', { name: 'Delete' }).click();
  await expect(dialog.getByRole('alert')).toBeVisible();
  expect(backend.conversations.has(conversationId)).toBe(true);
  await dialog.getByRole('button', { name: 'Delete' }).click();
  await expect(dialog).toHaveCount(0);
  expect(backend.conversations.has(conversationId)).toBe(false);

  backend.returnDeletedSummary = true;
  await page.getByLabel('Question').fill('Tombstone reload trigger');
  await page.getByRole('button', { name: 'Send' }).click();
  await page
    .getByRole('dialog', { name: 'Confirm learning request' })
    .getByRole('button', { name: 'Authorize and continue' })
    .click();
  const triggerRequest = lastRequest(backend);
  await waitForChannel(page, triggerRequest);
  await emit(page, backend, triggerRequest, {
    type: 'text_delta',
    text: 'Second safe answer',
  });
  await emit(page, backend, triggerRequest, { type: 'completed' });
  await expect(
    page.getByRole('button', { name: 'Tombstone reload trigger' }),
  ).toBeVisible();
  await expect(
    page.getByRole('button', { name: 'Durable safe question' }),
  ).toHaveCount(0);
  expect(backend.restartCount).toBe(1);
  await assertAccessibleAndPrivate(page, backend);
});

async function installBackend(page: Page, backend: SyntheticBookBackend) {
  page.on('pageerror', (error) => backend.consoleErrors.push(error.message));
  page.on('request', (request) => {
    const url = new URL(request.url());
    if (
      ['http:', 'https:'].includes(url.protocol) &&
      url.origin !== 'http://127.0.0.1:1420'
    ) {
      backend.externalOrigins.add(url.origin);
    }
  });
  await page.exposeFunction(
    '__phase14Invoke',
    (command: string, payload?: Record<string, unknown>) =>
      backend.invoke(command, payload),
  );
  await page.addInitScript(() => {
    type ChannelLike = { onmessage(payload: unknown): void };
    type Envelope = { __p14Response: unknown; __p14Events: unknown[] };
    type TestWindow = Window & {
      __phase14Invoke(
        command: string,
        payload?: Record<string, unknown>,
      ): Promise<unknown>;
      __phase14Emit(requestId: string, event: unknown): void;
      __phase14HasChannel(requestId: string): boolean;
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
    const channels = new Map<string, ChannelLike>();
    let callbackId = 1;
    target.__phase14Emit = (requestId, event) => {
      const channel = channels.get(requestId);
      if (!channel) throw new Error('missing synthetic request channel');
      channel.onmessage(event);
    };
    target.__phase14HasChannel = (requestId) => channels.has(requestId);
    target.__TAURI_INTERNALS__ = {
      transformCallback() {
        return callbackId++;
      },
      unregisterCallback() {},
      async invoke(command, payload = {}) {
        let safePayload = payload;
        if (command === 'subscribe_learning_request') {
          const channel = payload.events as ChannelLike;
          channels.set(String(payload.requestId), channel);
          safePayload = {
            requestId: payload.requestId,
            afterSeq: payload.afterSeq,
          };
        }
        const value = await target.__phase14Invoke(command, safePayload);
        if (value && typeof value === 'object' && '__p14Events' in value) {
          const envelope = value as Envelope;
          for (const event of envelope.__p14Events) {
            target.__phase14Emit(
              String((event as { requestId?: unknown }).requestId),
              event,
            );
          }
          return envelope.__p14Response;
        }
        return value;
      },
    };
  });
}

async function emit(
  page: Page,
  backend: SyntheticBookBackend,
  requestId: string,
  step: Parameters<SyntheticBookBackend['advance']>[1],
) {
  const event = backend.advance(requestId, step);
  await page.evaluate(
    ({ id, value }) =>
      (
        window as unknown as {
          __phase14Emit(requestId: string, event: unknown): void;
        }
      ).__phase14Emit(id, value),
    { id: requestId, value: event },
  );
}

async function waitForChannel(page: Page, requestId: string) {
  await expect
    .poll(() =>
      page.evaluate(
        (id) =>
          (
            window as unknown as {
              __phase14HasChannel(requestId: string): boolean;
            }
          ).__phase14HasChannel(id),
        requestId,
      ),
    )
    .toBe(true);
}

function lastRequest(backend: SyntheticBookBackend): string {
  const value = [...backend.requests.keys()].at(-1);
  if (!value) throw new Error('missing synthetic request');
  return value;
}

async function navigateSpa(page: Page, path: string) {
  await page.evaluate((next) => {
    history.pushState({}, '', next);
    dispatchEvent(new PopStateEvent('popstate'));
  }, path);
  await expect.poll(() => new URL(page.url()).pathname).toBe(path);
}

async function assertAccessibleAndPrivate(
  page: Page,
  backend: SyntheticBookBackend,
  allowedVisibleAnswers: readonly string[] = [],
) {
  const axe = await new AxeBuilder({ page })
    .disableRules(['color-contrast'])
    .analyze();
  expect(
    axe.violations.filter((violation) =>
      ['critical', 'serious'].includes(violation.impact ?? ''),
    ),
  ).toEqual([]);
  expect(backend.externalOrigins).toEqual(new Set());
  expect(backend.consoleErrors).toEqual([]);
  const surfaces = await page.evaluate(() =>
    JSON.stringify({
      localStorage: { ...localStorage },
      sessionStorage: { ...sessionStorage },
    }),
  );
  for (const forbidden of [
    PRIVATE_KEY,
    PRIVATE_PATH,
    PRIVATE_IMAGE,
    PRIVATE_PROMPT,
    PRIVATE_REASONING,
  ]) {
    expect(surfaces).not.toContain(forbidden);
  }
  for (const answer of allowedVisibleAnswers) {
    expect(page.locator('body')).toContainText(answer.split('\n')[0]!);
  }
}
