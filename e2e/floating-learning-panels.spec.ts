import AxeBuilder from '@axe-core/playwright';
import { expect, test, type Locator, type Page } from '@playwright/test';

const BOOK_ID = '10000000-0000-4000-8000-000000000001';
const SECTION_ID = '10000000-0000-4000-8000-000000000002';
const BLOCK_ALPHA = '10000000-0000-4000-8000-000000000003';
const BLOCK_BETA = '10000000-0000-4000-8000-000000000004';
const OLD_PROFILE = '10000000-0000-4000-8000-000000000005';
const NEW_PROFILE = '10000000-0000-4000-8000-000000000006';
const ALPHA = 'Concurrent alpha selection stays isolated.';
const BETA = 'Concurrent beta selection completes independently.';
const PARTIAL_SENTINEL = 'P12_INCOMPLETE_MEMORY_ONLY_SENTINEL';
const PRIVATE_SENTINEL = 'P12_PRIVATE_REASONING_SENTINEL';
const SCREENSHOT_SENTINEL = 'P12_SCREENSHOT_SENTINEL';
const FIXTURE_TIME = '2026-08-06T00:00:00.000Z';

type RequestStatus =
  'preparing' | 'streaming' | 'completed' | 'failed' | 'cancelled';
type RequestState = {
  requestId: string;
  preparationId: string | null;
  targetConversationId: string | null;
  question: string;
  profileId: string;
  modelId: string;
  status: RequestStatus;
  text: string;
  lastSeq: number;
  conversationId: string | null;
  metadata: Record<string, unknown> | null;
};
type Conversation = {
  id: string;
  bookId: string;
  annotationId: string;
  sectionId: string;
  anchor: Record<string, unknown>;
  selectedText: string | null;
  status: 'completed';
  messages: Array<Record<string, unknown>>;
  createdAt: string;
  updatedAt: string;
};
type TransportEnvelope = {
  __phase12Response: unknown;
  __phase12Events: Array<Record<string, unknown>>;
};

class SyntheticLearningBackend {
  readonly calls = new Map<string, number>();
  readonly externalOrigins = new Set<string>();
  readonly consoleErrors: string[] = [];
  readonly preparations = new Map<string, Record<string, unknown>>();
  readonly requests = new Map<string, RequestState>();
  readonly conversations = new Map<string, Conversation>();
  cancellationCalls = 0;
  successfulTransactions = 0;
  restartCount = 0;
  currentProfile = { id: OLD_PROFILE, modelId: 'old-model' };
  followupCaptures: Array<{ profileId: string; modelId: string }> = [];
  private nextId = 100;

  invoke(command: string, payload: Record<string, unknown> = {}): unknown {
    this.calls.set(command, (this.calls.get(command) ?? 0) + 1);
    switch (command) {
      case 'get_app_settings':
      case 'initialize_ui_language':
        return this.settings();
      case 'get_reader_settings':
        return {
          fontScale: 1,
          lineHeight: 1.6,
          readerWidth: 72,
          pdfZoom: 1,
          theme: 'system',
        };
      case 'get_reader_bootstrap':
        return this.bootstrap();
      case 'list_reader_sections':
        return [this.section()];
      case 'read_derived_text':
        return this.documentHtml();
      case 'list_provider_profiles':
        return [this.profile()];
      case 'get_teaching_instruction':
        return {
          instruction: '',
          revision: 0,
          updatedAt: FIXTURE_TIME,
        };
      case 'list_annotation_markers':
        return this.markers();
      case 'prepare_learning_request':
        return this.prepare(payload.metadata as Record<string, unknown>);
      case 'start_learning_request':
        return this.start(String(payload.preparationId));
      case 'subscribe_learning_request':
        return this.subscribe(
          String(payload.requestId),
          Number(payload.afterSeq),
        );
      case 'cancel_learning_request':
        return this.cancel(String(payload.requestId));
      case 'get_learning_conversation':
        return this.getConversation(
          String(payload.bookId),
          String(payload.conversationId),
        );
      case 'start_conversation_followup':
        return this.startFollowup(
          String(payload.conversationId),
          String(payload.question),
        );
      case 'delete_learning_conversation':
        return this.deleteConversation(
          String(payload.bookId),
          String(payload.conversationId),
          String(payload.annotationId),
        );
      case 'save_reading_progress':
      case 'complete_first_reader_hint':
        return undefined;
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
    if (request.status === 'cancelled' || request.status === 'completed') {
      throw new Error('synthetic post-terminal event rejected');
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
    if (!request.text.trim()) throw new Error('empty completion rejected');
    const conversationId = this.commit(request);
    request.status = 'completed';
    request.conversationId = conversationId;
    return {
      requestId,
      seq: request.lastSeq,
      event: { type: 'completed', conversationId },
    };
  }

  restart(): void {
    this.restartCount += 1;
    for (const request of this.requests.values()) {
      if (request.status !== 'completed') {
        request.text = '';
      }
    }
    this.requests.clear();
  }

  safeStats() {
    return {
      calls: Object.fromEntries(this.calls),
      cancellations: this.cancellationCalls,
      transactions: this.successfulTransactions,
      conversations: this.conversations.size,
      messages: [...this.conversations.values()].reduce(
        (count, conversation) => count + conversation.messages.length,
        0,
      ),
      markers: this.markers().length,
      restartCount: this.restartCount,
      followupCaptures: this.followupCaptures,
    };
  }

  private settings() {
    return {
      onboardingCompleted: true,
      activeProviderProfileId: this.currentProfile.id,
      defaultLearningProfileId: this.currentProfile.id,
      defaultVisionProfileId: this.currentProfile.id,
      theme: 'system',
      contextMode: 'standard',
      uiLanguage: 'en',
      uiLanguageInitialized: true,
      firstReaderHintCompleted: true,
    };
  }

  private bootstrap() {
    return {
      book: {
        id: BOOK_ID,
        title: 'Synthetic concurrent learning book',
        originalFilename: 'synthetic.docx',
        author: null,
        language: 'en',
        format: 'docx',
        importStatus: 'ready',
        importErrorCode: null,
        importErrorMessage: null,
        importErrorStage: null,
        readingProgress: 0,
        createdAt: FIXTURE_TIME,
        updatedAt: FIXTURE_TIME,
        lastOpenedAt: null,
      },
      lastLocator: null,
    };
  }

  private section() {
    return {
      id: SECTION_ID,
      parentId: null,
      ordinal: 0,
      title: 'Synthetic section',
      locator: {
        format: 'docx',
        startBlockId: BLOCK_ALPHA,
        startOffset: 0,
        endBlockId: BLOCK_BETA,
        endOffset: 0,
      },
    };
  }

  private documentHtml() {
    return [
      `<p data-section-id="${SECTION_ID}" data-block-id="${BLOCK_ALPHA}">${ALPHA}</p>`,
      `<p data-section-id="${SECTION_ID}" data-block-id="${BLOCK_BETA}">${BETA}</p>`,
    ].join('');
  }

  private profile() {
    return {
      id: this.currentProfile.id,
      kind: 'openai',
      displayName:
        this.currentProfile.id === OLD_PROFILE
          ? 'Old profile'
          : 'Current profile',
      modelId: this.currentProfile.modelId,
      contextWindowTokens: 32_000,
      isActive: true,
      credentialStatus: 'available',
      validatedAt: FIXTURE_TIME,
    };
  }

  private markers() {
    return [...this.conversations.values()].map((conversation) => ({
      id: conversation.annotationId,
      kind: 'ai_conversation',
      conversationId: conversation.id,
      anchor: conversation.anchor,
      relocationStatus: 'primary',
      accessibilityLabel: 'View AI conversation marker',
    }));
  }

  private prepare(metadata: Record<string, unknown>) {
    const preparationId = this.uuid();
    this.preparations.set(preparationId, structuredClone(metadata));
    return {
      preparationId,
      providerDisplayName: 'Synthetic provider',
      profileDisplayName: 'Synthetic profile',
      modelDisplayName: String(metadata.modelId),
      estimatedInputTokens: 24,
      sourceCount: 1,
      citationCount: 0,
      omittedSourceCount: 0,
      willSendImage: false,
      riskFlags: [],
      requiresBlockingConfirmation: false,
      expiresAt: '2026-08-06T01:00:00.000Z',
      actionCategory: metadata.action,
    };
  }

  private start(preparationId: string) {
    const metadata = this.preparations.get(preparationId);
    if (!metadata) throw { code: 'NOT_FOUND' };
    this.preparations.delete(preparationId);
    const requestId = this.uuid();
    const request: RequestState = {
      requestId,
      preparationId,
      targetConversationId: null,
      question: 'Explain the selected textbook content.',
      profileId: String(metadata.providerProfileId),
      modelId: String(metadata.modelId),
      status: 'preparing',
      text: '',
      lastSeq: 1,
      conversationId: null,
      metadata: structuredClone(metadata),
    };
    this.requests.set(requestId, request);
    return this.snapshot(request);
  }

  private subscribe(requestId: string, afterSeq: number) {
    const request = this.request(requestId);
    if (afterSeq > request.lastSeq) throw { code: 'INVALID_INPUT' };
    return this.snapshot(request);
  }

  private cancel(requestId: string): TransportEnvelope {
    const request = this.request(requestId);
    if (request.status === 'completed' || request.status === 'cancelled') {
      return { __phase12Response: null, __phase12Events: [] };
    }
    this.cancellationCalls += 1;
    request.status = 'cancelled';
    request.lastSeq += 1;
    return {
      __phase12Response: null,
      __phase12Events: [
        {
          requestId,
          seq: request.lastSeq,
          event: { type: 'cancelled' },
        },
      ],
    };
  }

  private getConversation(bookId: string, conversationId: string) {
    const conversation = this.conversations.get(conversationId);
    if (!conversation || conversation.bookId !== bookId)
      throw { code: 'NOT_FOUND' };
    return structuredClone(conversation);
  }

  private startFollowup(conversationId: string, question: string) {
    if (!this.conversations.has(conversationId)) throw { code: 'NOT_FOUND' };
    const active = [...this.requests.values()].some(
      (request) =>
        request.targetConversationId === conversationId &&
        !['completed', 'failed', 'cancelled'].includes(request.status),
    );
    if (active) throw { code: 'REQUEST_CONFLICT' };
    const requestId = this.uuid();
    const request: RequestState = {
      requestId,
      preparationId: null,
      targetConversationId: conversationId,
      question,
      profileId: this.currentProfile.id,
      modelId: this.currentProfile.modelId,
      status: 'preparing',
      text: '',
      lastSeq: 1,
      conversationId: null,
      metadata: null,
    };
    this.followupCaptures.push({
      profileId: request.profileId,
      modelId: request.modelId,
    });
    this.requests.set(requestId, request);
    return this.snapshot(request);
  }

  private deleteConversation(
    bookId: string,
    conversationId: string,
    annotationId: string,
  ) {
    const conversation = this.conversations.get(conversationId);
    if (
      !conversation ||
      conversation.bookId !== bookId ||
      conversation.annotationId !== annotationId
    ) {
      throw { code: 'NOT_FOUND' };
    }
    this.conversations.delete(conversationId);
    return undefined;
  }

  private commit(request: RequestState): string {
    if (request.targetConversationId) {
      const conversation = this.conversations.get(request.targetConversationId);
      if (!conversation) throw new Error('missing follow-up owner');
      const ordinal = conversation.messages.length;
      conversation.messages.push(
        this.message(ordinal, 'user', request.question, null, null),
        this.message(
          ordinal + 1,
          'assistant',
          request.text,
          request.profileId,
          request.modelId,
        ),
      );
      conversation.updatedAt = '2026-08-06T00:01:00.000Z';
      this.successfulTransactions += 1;
      return conversation.id;
    }
    const metadata = this.preparationMetadata(request);
    const conversationId = this.uuid();
    const conversation: Conversation = {
      id: conversationId,
      bookId: BOOK_ID,
      annotationId: this.uuid(),
      sectionId: SECTION_ID,
      anchor: structuredClone(metadata.anchor as Record<string, unknown>),
      selectedText: String(metadata.selectedText),
      status: 'completed',
      messages: [
        this.message(0, 'user', request.question, null, null),
        this.message(
          1,
          'assistant',
          request.text,
          request.profileId,
          request.modelId,
        ),
      ],
      createdAt: FIXTURE_TIME,
      updatedAt: FIXTURE_TIME,
    };
    this.conversations.set(conversationId, conversation);
    this.successfulTransactions += 1;
    return conversationId;
  }

  private preparationMetadata(request: RequestState) {
    if (!request.metadata)
      throw new Error('missing immutable preparation metadata');
    return request.metadata;
  }

  private message(
    ordinal: number,
    role: 'user' | 'assistant',
    content: string,
    providerId: string | null,
    modelId: string | null,
  ) {
    return {
      id: this.uuid(),
      ordinal,
      role,
      action: ordinal < 2 ? 'explain' : 'continue',
      content,
      providerId,
      modelId,
      citations: [],
      createdAt: FIXTURE_TIME,
    };
  }

  private request(requestId: string) {
    const request = this.requests.get(requestId);
    if (!request) throw { code: 'NOT_FOUND' };
    return request;
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

  private uuid() {
    return `20000000-0000-4000-8000-${String(this.nextId++).padStart(12, '0')}`;
  }
}

/* eslint-disable react-hooks/rules-of-hooks -- Playwright fixture callback. */
const learningTest = test.extend<{ backend: SyntheticLearningBackend }>({
  backend: async ({ page }, use) => {
    const backend = new SyntheticLearningBackend();
    await installMock(page, backend);
    await use(backend);
  },
});
/* eslint-enable react-hooks/rules-of-hooks */

learningTest(
  'G: two real product requests move resize reorder and stop independently',
  async ({ page, backend }) => {
    await openReader(page);
    const alphaRequest = await startSelection(
      page,
      backend,
      BLOCK_ALPHA,
      ALPHA,
    );
    const betaRequest = await startSelection(page, backend, BLOCK_BETA, BETA);
    await expect(page.locator('[data-panel-id]')).toHaveCount(2);

    await emit(page, backend, alphaRequest, {
      type: 'text_delta',
      text: 'alpha partial',
    });
    await emit(page, backend, betaRequest, {
      type: 'text_delta',
      text: 'beta partial',
    });
    const alphaPanel = panelForAnswer(page, 'alpha partial');
    const betaPanel = panelForAnswer(page, 'beta partial');
    await expect(alphaPanel).toContainText('alpha partial');
    await expect(betaPanel).toContainText('beta partial');

    const alphaInitialZ = await zIndex(alphaPanel);
    const betaInitialZ = await zIndex(betaPanel);
    expect(betaInitialZ).toBeGreaterThan(alphaInitialZ);
    const alphaBefore = await box(alphaPanel);
    const betaBefore = await box(betaPanel);
    await dragBy(alphaPanel.getByLabel('Move learning panel'), -140, -90);
    const alphaMoved = await box(alphaPanel);
    const betaUnchanged = await box(betaPanel);
    expect(alphaMoved.x).not.toBe(alphaBefore.x);
    expect(alphaMoved.y).not.toBe(alphaBefore.y);
    expect(betaUnchanged).toEqual(betaBefore);

    const alphaResize = alphaPanel.getByRole('button', { name: 'Resize se' });
    await alphaResize.focus();
    await alphaResize.press('ArrowRight');
    await alphaResize.press('ArrowDown');
    const alphaResized = await box(alphaPanel);
    expect(alphaResized.width).toBeGreaterThan(alphaMoved.width);
    expect(alphaResized.height).toBeGreaterThan(alphaMoved.height);
    expect(await box(betaPanel)).toEqual(betaBefore);

    expect(await zIndex(alphaPanel)).toBeGreaterThan(await zIndex(betaPanel));

    await alphaPanel.getByRole('button', { name: 'Stop' }).click();
    await expect(alphaPanel).toContainText('cancelled');
    await emit(page, backend, betaRequest, {
      type: 'text_delta',
      text: ' and durable',
    });
    await emit(page, backend, betaRequest, { type: 'completed' });
    await expect(betaPanel).toContainText('completed');
    await expect(betaPanel).toContainText('beta partial and durable');
    expect(backend.safeStats()).toMatchObject({
      cancellations: 1,
      transactions: 1,
      conversations: 1,
      messages: 2,
      markers: 1,
    });
    await assertAccessibleAndLocal(page, backend);
  },
);

learningTest(
  'H: collapse and hide keep the stream alive, safe, and reopenable',
  async ({ page, backend }) => {
    await openReader(page);
    const requestId = await startSelection(page, backend, BLOCK_ALPHA, ALPHA);
    await emit(page, backend, requestId, {
      type: 'text_delta',
      text: 'Safe partial $x',
    });
    const streamedPanel = panelForAnswer(page, 'Safe partial');
    await expect(streamedPanel).toBeVisible();
    const streamingPanelId = await streamedPanel.getAttribute('data-panel-id');
    expect(streamingPanelId).not.toBeNull();
    const panel = page.locator(`[data-panel-id="${streamingPanelId}"]`);
    await panel.getByRole('button', { name: 'Collapse' }).click();
    await expect(panel.getByRole('button', { name: 'Expand' })).toBeVisible();
    await emit(page, backend, requestId, {
      type: 'text_delta',
      text: '$\n\n[js](javascript:alert(1))\n\n![x](https://invalid.example/x.png)\n\n<script>unsafe()</script>\n\n<think>P12_PRIVATE_REASONING_SENTINEL</think>',
    });
    expect(backend.cancellationCalls).toBe(0);
    await panel.getByRole('button', { name: 'Expand' }).click();
    await expect(panel).toContainText('Safe partial');
    await expect(panel).toContainText('link blocked');
    await expect(panel.locator('script,img')).toHaveCount(0);
    await expect(
      panel.locator('a[href^="javascript:"],a[href^="data:"],a[href^="file:"]'),
    ).toHaveCount(0);
    await expect(panel).not.toContainText(PRIVATE_SENTINEL);

    await panel.getByRole('button', { name: 'Hide' }).click();
    await expect(panel).toHaveCount(0);
    await emit(page, backend, requestId, {
      type: 'text_delta',
      text: '\nDurable result.',
    });
    await emit(page, backend, requestId, { type: 'completed' });
    expect(backend.safeStats()).toMatchObject({
      cancellations: 0,
      transactions: 1,
      conversations: 1,
      markers: 1,
    });

    await navigateSpa(page, '/teaching-instructions');
    await navigateSpa(page, `/books/${BOOK_ID}/read`);
    await expect(page.locator('[data-panel-id]')).toHaveCount(0);
    const marker = historyMarker(page);
    await expect(marker).toBeVisible();
    await marker.click();
    const reopened = page.locator('[data-panel-id]');
    await expect(reopened).toHaveCount(1);
    await expect(reopened).toContainText('Durable result.');
    const panelId = await reopened.getAttribute('data-panel-id');
    const firstZ = await zIndex(reopened);
    await reopened.getByRole('button', { name: 'Hide' }).click();
    await marker.click();
    await expect(page.locator('[data-panel-id]')).toHaveCount(1);
    expect(await reopened.getAttribute('data-panel-id')).toBe(panelId);
    expect(await zIndex(reopened)).toBeGreaterThan(firstZ);

    const screenshot = await page.screenshot();
    expect(screenshot.toString('utf8')).not.toContain(SCREENSHOT_SENTINEL);
    await assertAccessibleAndLocal(page, backend);
  },
);

learningTest(
  'I: restart restores only durable history, captures current followup profile, and deletes atomically',
  async ({ page, backend }) => {
    await openReader(page);
    const incomplete = await startSelection(page, backend, BLOCK_ALPHA, ALPHA);
    await emit(page, backend, incomplete, {
      type: 'text_delta',
      text: PARTIAL_SENTINEL,
    });
    await panelForAnswer(page, PARTIAL_SENTINEL)
      .getByRole('button', { name: 'Hide' })
      .click();

    const completed = await startSelection(page, backend, BLOCK_BETA, BETA);
    await emit(page, backend, completed, {
      type: 'text_delta',
      text: 'Durable restart answer.',
    });
    await emit(page, backend, completed, { type: 'completed' });
    expect(backend.conversations.size).toBe(1);
    backend.restart();
    await page.reload();
    await expect(page.getByText(ALPHA, { exact: false })).toBeVisible();
    await expect(page.locator('[data-panel-id]')).toHaveCount(0);
    await expect(page.getByText(PARTIAL_SENTINEL)).toHaveCount(0);

    const marker = historyMarker(page);
    await expect(marker).toBeVisible();
    await marker.click();
    const historyPanel = page.locator('[data-panel-id]');
    await expect(historyPanel).toHaveCount(1);
    await expect(historyPanel).toContainText('Durable restart answer.');
    await expect(historyPanel).toContainText(`Provider ${OLD_PROFILE}`);
    await expect(historyPanel).toContainText('Model old-model');

    backend.currentProfile = { id: NEW_PROFILE, modelId: 'current-model' };
    await historyPanel
      .getByRole('textbox', { name: 'Follow up' })
      .fill('Use the current profile now.');
    await historyPanel.getByRole('button', { name: 'Send' }).click();
    const followup = [...backend.requests.values()].at(-1)!;
    expect(backend.followupCaptures).toEqual([
      { profileId: NEW_PROFILE, modelId: 'current-model' },
    ]);
    await emit(page, backend, followup.requestId, {
      type: 'text_delta',
      text: 'Current profile follow-up.',
    });
    await emit(page, backend, followup.requestId, { type: 'completed' });
    await expect(historyPanel).toContainText('Current profile follow-up.');
    await expect(historyPanel).toContainText(`Provider ${OLD_PROFILE}`);
    await expect(historyPanel).toContainText('Model old-model');
    await expect(historyPanel).toContainText(`Provider ${NEW_PROFILE}`);
    await expect(historyPanel).toContainText('Model current-model');

    page.once('dialog', (dialog) => dialog.accept());
    await historyPanel.getByRole('button', { name: 'Delete' }).click();
    await expect(page.locator('[data-panel-id]')).toHaveCount(0);
    await expect(marker).toHaveCount(0);
    expect(backend.safeStats()).toMatchObject({
      cancellations: 0,
      transactions: 2,
      conversations: 0,
      messages: 0,
      markers: 0,
      restartCount: 1,
    });
    expect(JSON.stringify(backend.safeStats())).not.toContain(PARTIAL_SENTINEL);
    await assertAccessibleAndLocal(page, backend);
  },
);

async function installMock(page: Page, backend: SyntheticLearningBackend) {
  page.on('console', (message) => {
    if (message.type() === 'error') backend.consoleErrors.push(message.text());
  });
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
    '__phase12Invoke',
    (command: string, payload?: Record<string, unknown>) =>
      backend.invoke(command, payload),
  );
  await page.addInitScript(() => {
    type ChannelLike = { onmessage(payload: unknown): void };
    type Envelope = {
      __phase12Response: unknown;
      __phase12Events: unknown[];
    };
    type TestWindow = Window & {
      __phase12Invoke(
        command: string,
        payload?: Record<string, unknown>,
      ): Promise<unknown>;
      __phase12Emit(requestId: string, event: unknown): void;
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
    target.__phase12Emit = (requestId, event) => {
      const channel = channels.get(requestId);
      if (!channel) throw new Error('missing synthetic request channel');
      channel.onmessage(event);
    };
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
        const value = await target.__phase12Invoke(command, safePayload);
        if (value && typeof value === 'object' && '__phase12Events' in value) {
          const envelope = value as Envelope;
          for (const event of envelope.__phase12Events) {
            const requestId = String(
              (event as { requestId?: unknown }).requestId,
            );
            target.__phase12Emit(requestId, event);
          }
          return envelope.__phase12Response;
        }
        return value;
      },
    };
  });
}

async function openReader(page: Page) {
  await page.goto(`/books/${BOOK_ID}/read`);
  await expect(page.getByText(ALPHA, { exact: false })).toBeVisible();
}

async function startSelection(
  page: Page,
  backend: SyntheticLearningBackend,
  blockId: string,
  text: string,
) {
  const before = backend.requests.size;
  await selectText(page, blockId, text);
  await page.getByRole('menuitem', { name: 'Explain' }).click();
  await expect.poll(() => backend.requests.size).toBe(before + 1);
  return [...backend.requests.keys()].at(-1)!;
}

async function selectText(page: Page, blockId: string, text: string) {
  await page.evaluate(
    ({ id, expected }) => {
      const block = document.querySelector(`[data-block-id="${id}"]`);
      const node = block?.firstChild;
      if (!(node instanceof Text) || node.data !== expected) {
        throw new Error('missing synthetic selection block');
      }
      const range = document.createRange();
      range.setStart(node, 0);
      range.setEnd(node, node.length);
      const selection = window.getSelection();
      selection?.removeAllRanges();
      selection?.addRange(range);
      block.dispatchEvent(new MouseEvent('mouseup', { bubbles: true }));
    },
    { id: blockId, expected: text },
  );
  await expect(
    page.getByRole('menu', { name: 'Learning actions' }),
  ).toBeVisible();
}

async function emit(
  page: Page,
  backend: SyntheticLearningBackend,
  requestId: string,
  step: Parameters<SyntheticLearningBackend['advance']>[1],
) {
  const event = backend.advance(requestId, step);
  await page.evaluate(
    ({ id, value }) =>
      (
        window as unknown as {
          __phase12Emit(requestId: string, event: unknown): void;
        }
      ).__phase12Emit(id, value),
    { id: requestId, value: event },
  );
}

function panelForAnswer(page: Page, answer: string) {
  return page.locator('[data-panel-id]').filter({ hasText: answer });
}

async function dragBy(locator: Locator, x: number, y: number) {
  const bounds = await box(locator);
  const startX = bounds.x + Math.min(30, bounds.width / 2);
  const startY = bounds.y + Math.min(20, bounds.height / 2);
  await locator.evaluate(
    (element, movement) => {
      element.dispatchEvent(
        new PointerEvent('pointerdown', {
          bubbles: true,
          clientX: movement.startX,
          clientY: movement.startY,
          pointerId: 1,
        }),
      );
      window.dispatchEvent(
        new PointerEvent('pointermove', {
          bubbles: true,
          clientX: movement.startX + movement.x,
          clientY: movement.startY + movement.y,
          pointerId: 1,
        }),
      );
      window.dispatchEvent(
        new PointerEvent('pointerup', {
          bubbles: true,
          clientX: movement.startX + movement.x,
          clientY: movement.startY + movement.y,
          pointerId: 1,
        }),
      );
    },
    { startX, startY, x, y },
  );
}

function historyMarker(page: Page) {
  return page
    .getByRole('complementary', { name: 'Marker history' })
    .getByRole('button', { name: 'Open answer' });
}

async function box(locator: Locator) {
  const value = await locator.boundingBox();
  if (!value) throw new Error('expected visible synthetic element');
  return value;
}

async function zIndex(locator: Locator) {
  return locator.evaluate((element) =>
    Number(getComputedStyle(element).zIndex),
  );
}

async function navigateSpa(page: Page, path: string) {
  await page.evaluate((next) => {
    history.pushState({}, '', next);
    window.dispatchEvent(new PopStateEvent('popstate'));
  }, path);
  await expect.poll(() => new URL(page.url()).pathname).toBe(path);
  if (path.includes('/books/')) {
    await expect(page.getByText(ALPHA, { exact: false })).toBeVisible();
  }
}

async function assertAccessibleAndLocal(
  page: Page,
  backend: SyntheticLearningBackend,
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
      body: document.body.innerHTML,
      localStorage: { ...localStorage },
      sessionStorage: { ...sessionStorage },
    }),
  );
  for (const sentinel of [
    PARTIAL_SENTINEL,
    PRIVATE_SENTINEL,
    SCREENSHOT_SENTINEL,
  ]) {
    expect(surfaces).not.toContain(sentinel);
  }
}
