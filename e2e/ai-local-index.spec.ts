import AxeBuilder from '@axe-core/playwright';
import { expect, test as base, type Page } from '@playwright/test';
import { readFile } from 'node:fs/promises';
import { PDFDocument } from 'pdf-lib';

interface MockEnvelope {
  ok: boolean;
  value?: unknown;
  error?: Record<string, unknown>;
}

interface SafeStats {
  calls: Record<string, number>;
  claimCounts: Record<string, number>;
  committedPages: number[];
  providerDispatches: number;
  uploads: number;
  uploadedPageBatches: number[][];
  confirmedPages: Array<{ pageNumber: number; qualityReason: string }>;
  remoteResources: number;
  retryExpectedVersion: string | null;
  sourceReadsAfterRemoval: number;
}

const bookId = '00000000-0000-4000-8000-000000000901';
const profileId = '00000000-0000-4000-8000-000000000902';
const runId = '00000000-0000-4000-8000-000000000903';
const pageIds = {
  1: '00000000-0000-4000-8000-000000000911',
  2: '00000000-0000-4000-8000-000000000912',
} as const;
const firstAttempts = {
  1: '00000000-0000-4000-8000-000000000921',
  2: '00000000-0000-4000-8000-000000000922',
} as const;
const retryAttempt = '00000000-0000-4000-8000-000000000923';
const failedUpdatedAt = '2026-08-04T00:00:01.000Z';
const readyUpdatedAt = '2026-08-04T00:00:02.000Z';
const fixedTimestamp = '2026-08-04T00:00:00.000Z';
const committedText = 'durable optics survives local restart';
const exactLimitation =
  'This PDF has unreliable or no local text. Search and text-based AI are limited; local page reading remains available.';
const forbiddenBrowserSentinels = [
  'credential-private-sentinel-p9',
  'opaque-remote-private-sentinel-p9',
  'provider-prompt-private-sentinel-p9',
  'source-textbook-private-sentinel-p9',
] as const;

class SyntheticAiIndexBackend {
  readonly calls: string[] = [];
  readonly claimCounts = new Map<number, number>();
  readonly committedPages = new Set<number>();
  readonly consoleMessages: string[] = [];
  readonly externalOrigins = new Set<string>();
  readonly source: Uint8Array;
  failFirstPageOnce: boolean;

  private runCreated = false;
  private initialClaimed = false;
  private retryQueued = false;
  private retryClaimed = false;
  private firstSubmissionFinished = false;
  private dependenciesRemoved = false;
  private providerDispatches = 0;
  private uploads = 0;
  private readonly uploadedPageBatches: number[][] = [];
  private confirmedPages: Array<{
    pageNumber: number;
    qualityReason: string;
  }> = [];
  private remoteResources = 0;
  private sourceReadsAfterRemoval = 0;
  private retryExpectedVersion: string | null = null;

  constructor(
    source: Uint8Array,
    options: { failFirstPageOnce?: boolean } = {},
  ) {
    this.source = source;
    this.failFirstPageOnce = options.failFirstPageOnce ?? false;
  }

  removeTransientDependencies() {
    this.dependenciesRemoved = true;
  }

  safeStats(): SafeStats {
    return {
      calls: Object.fromEntries(
        [...new Set(this.calls)].map((command) => [
          command,
          this.calls.filter((candidate) => candidate === command).length,
        ]),
      ),
      claimCounts: Object.fromEntries(this.claimCounts),
      committedPages: [...this.committedPages].sort(),
      providerDispatches: this.providerDispatches,
      uploads: this.uploads,
      uploadedPageBatches: this.uploadedPageBatches.map((batch) => [...batch]),
      confirmedPages: this.confirmedPages.map((page) => ({ ...page })),
      remoteResources: this.remoteResources,
      retryExpectedVersion: this.retryExpectedVersion,
      sourceReadsAfterRemoval: this.sourceReadsAfterRemoval,
    };
  }

  async invoke(
    command: string,
    payload: Record<string, unknown> = {},
  ): Promise<MockEnvelope> {
    this.calls.push(command);
    switch (command) {
      case 'get_app_settings':
      case 'initialize_ui_language':
      case 'update_ui_language':
        return this.success(this.settings());
      case 'get_onboarding_state':
        return this.success({
          step: 'ready',
          selectedBook: null,
          hasReadyBook: true,
          learningProfileConnected: false,
          visionProfileConnected: true,
          localTextQuality: 'needs_ocr',
          canSkipOnboarding: true,
        });
      case 'list_books':
        return this.success([this.book()]);
      case 'get_reader_bootstrap':
        return this.success({ book: this.book(), lastLocator: null });
      case 'get_reader_settings':
      case 'update_reader_settings':
        return this.success({
          fontScale: 1,
          lineHeight: 1.6,
          readerWidth: 72,
          pdfZoom: 1,
          theme: 'system',
        });
      case 'list_reader_sections':
        return this.success([
          {
            id: '00000000-0000-4000-8000-000000000931',
            parentId: null,
            ordinal: 0,
            title: 'Synthetic page 1',
            locator: this.locator(1),
          },
        ]);
      case 'list_annotation_markers':
        return this.success([]);
      case 'read_book_source':
      case 'read_claimed_index_source':
        if (this.dependenciesRemoved) {
          this.sourceReadsAfterRemoval += 1;
          return this.failure('LOCAL_IO_ERROR');
        }
        return this.success({ __binary: [...this.source] });
      case 'list_provider_profiles':
        if (payload.operation !== 'structured_page_analysis')
          return this.failure('INVALID_INPUT');
        return this.success([this.profile()]);
      case 'confirm_index_operation':
        return this.confirmOperation(payload);
      case 'update_provider_operation_consent':
        return this.success(null);
      case 'create_index_run':
        return this.createRun(payload);
      case 'claim_index_render_batch':
        return this.success(this.claimBatch());
      case 'submit_index_render_batch':
        return this.submitBatch(payload);
      case 'report_index_render_failure':
        return this.failure('LOCAL_IO_ERROR');
      case 'get_index_run_aggregate':
        return this.success(this.aggregate());
      case 'find_current_index_run_for_book':
        return this.success(this.runCreated ? this.aggregate() : null);
      case 'list_index_page_reviews':
        return this.success(this.reviews());
      case 'retry_index_page':
        return this.retryPage(payload);
      case 'search_book':
        return this.search(payload);
      case 'save_reading_progress':
        return this.success(null);
      default:
        return this.failure('INVALID_INPUT');
    }
  }

  private settings() {
    return {
      onboardingCompleted: true,
      activeProviderProfileId: profileId,
      defaultLearningProfileId: null,
      defaultVisionProfileId: profileId,
      theme: 'system',
      contextMode: 'standard',
      uiLanguage: 'en',
      uiLanguageInitialized: true,
      firstReaderHintCompleted: true,
    };
  }

  private book() {
    return {
      id: bookId,
      title: 'Self-made blank-page textbook',
      originalFilename: 'self-made-pages.pdf',
      author: null,
      language: 'en',
      format: 'pdf',
      importStatus: 'ready',
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
      createdAt: fixedTimestamp,
      updatedAt: fixedTimestamp,
      lastOpenedAt: null,
    };
  }

  private profile() {
    return {
      id: profileId,
      kind: 'openai',
      displayName: 'Loopback vision profile',
      modelId: 'synthetic-vision-v1',
      contextWindowTokens: 32_000,
      isActive: true,
      credentialStatus: 'available',
      validatedAt: fixedTimestamp,
    };
  }

  private createRun(payload: Record<string, unknown>): MockEnvelope {
    const request = payload.request as {
      pages?: Array<{ pageNumber: number; qualityReason: string }>;
    };
    if (
      this.runCreated ||
      request.pages?.length !== 2 ||
      request.pages.some(
        (page) =>
          ![1, 2].includes(page.pageNumber) ||
          page.qualityReason === 'reliable_text',
      )
    ) {
      return this.failure('REQUEST_CONFLICT');
    }
    this.runCreated = true;
    return this.success(runId);
  }

  private confirmOperation(payload: Record<string, unknown>): MockEnvelope {
    const request = payload.request as {
      pages?: Array<{
        pageNumber: number;
        qualityReason: string;
        localTextSha256: string | null;
      }>;
    };
    const pages = request.pages ?? [];
    if (
      pages.length !== 2 ||
      pages.some(
        (page, index) =>
          page.pageNumber !== index + 1 ||
          page.qualityReason !== 'no_text' ||
          page.localTextSha256 !== null,
      )
    ) {
      return this.failure('INVALID_INPUT');
    }
    this.confirmedPages = pages.map(({ pageNumber, qualityReason }) => ({
      pageNumber,
      qualityReason,
    }));
    return this.success('00000000-0000-4000-8000-000000000941');
  }

  private claimBatch() {
    if (!this.runCreated) return { claims: [] };
    if (!this.initialClaimed) {
      this.initialClaimed = true;
      return {
        claims: [1, 2].map((pageNumber) => {
          this.incrementClaim(pageNumber);
          return this.claim(
            pageNumber as 1 | 2,
            firstAttempts[pageNumber as 1 | 2],
          );
        }),
      };
    }
    if (this.retryQueued && !this.retryClaimed) {
      this.retryClaimed = true;
      this.incrementClaim(1);
      return { claims: [this.claim(1, retryAttempt)] };
    }
    return { claims: [] };
  }

  private claim(pageNumber: 1 | 2, attemptId: string) {
    return {
      runId,
      bookId,
      pageId: pageIds[pageNumber],
      pageNumber,
      attemptId,
      limits: {
        maxDimension: 1024,
        maxDecodedPixels: 1_048_576,
        maxEncodedBytes: 1_048_576,
        maxTotalEncodedBytes: 2_097_152,
      },
    };
  }

  private submitBatch(payload: Record<string, unknown>): MockEnvelope {
    const captureCount = Number(payload.captureCount ?? 0);
    const encodedByteLength = Number(payload.encodedByteLength ?? 0);
    const pageNumbers = payload.pageNumbers as number[];
    if (
      captureCount < 1 ||
      captureCount > 2 ||
      encodedByteLength <= 0 ||
      !Array.isArray(pageNumbers) ||
      pageNumbers.length !== captureCount ||
      pageNumbers.some((pageNumber) => ![1, 2].includes(pageNumber))
    )
      return this.failure('INVALID_INPUT');
    this.uploads += 1;
    this.uploadedPageBatches.push([...pageNumbers]);
    this.providerDispatches += 1;
    if (!this.firstSubmissionFinished) {
      this.firstSubmissionFinished = true;
      this.committedPages.add(2);
      if (!this.failFirstPageOnce) this.committedPages.add(1);
      return this.success(
        [1, 2].map((pageNumber) => ({
          runId,
          pageId: pageIds[pageNumber as 1 | 2],
          status:
            this.failFirstPageOnce && pageNumber === 1 ? 'failed' : 'indexed',
          safeErrorCode:
            this.failFirstPageOnce && pageNumber === 1
              ? 'INDEX_PROVIDER_FAILED'
              : null,
        })),
      );
    }
    if (this.retryClaimed) {
      this.committedPages.add(1);
      return this.success([
        {
          runId,
          pageId: pageIds[1],
          status: 'indexed',
          safeErrorCode: null,
        },
      ]);
    }
    return this.failure('REQUEST_CONFLICT');
  }

  private aggregate() {
    const indexed = this.committedPages.size;
    const failed =
      this.failFirstPageOnce &&
      this.firstSubmissionFinished &&
      !this.committedPages.has(1)
        ? 1
        : 0;
    const queued = this.runCreated && !this.firstSubmissionFinished ? 2 : 0;
    return {
      runId,
      bookId,
      controlStatus: indexed === 2 ? 'completed' : 'running',
      aggregateStatus:
        indexed === 2 ? 'ready' : failed === 1 ? 'partial' : 'partial',
      pages: {
        total: this.runCreated ? 2 : 0,
        notRequired: 0,
        queued,
        rendering: 0,
        sending: 0,
        parsing: 0,
        validating: 0,
        indexed,
        needsReview: 0,
        failed,
        cancelled: 0,
      },
      updatedAt: indexed === 2 ? readyUpdatedAt : failedUpdatedAt,
    };
  }

  private reviews() {
    if (
      !this.failFirstPageOnce ||
      !this.firstSubmissionFinished ||
      this.committedPages.has(1)
    ) {
      return [];
    }
    return [
      {
        id: pageIds[1],
        runId,
        bookId,
        pageNumber: 1,
        qualityReason: 'no_text',
        status: 'failed',
        reviewReason: null,
        safeError: {
          code: 'INDEX_PROVIDER_FAILED',
          message: 'The page analysis provider did not complete.',
          retryable: true,
        },
        contentVersion: 1,
        blocks: [],
        corrections: [],
        updatedAt: failedUpdatedAt,
      },
    ];
  }

  private retryPage(payload: Record<string, unknown>): MockEnvelope {
    if (
      payload.pageId !== pageIds[1] ||
      payload.expectedUpdatedAt !== failedUpdatedAt ||
      this.retryQueued ||
      this.committedPages.has(1)
    ) {
      return this.failure('REQUEST_CONFLICT');
    }
    this.retryExpectedVersion = String(payload.expectedUpdatedAt);
    this.retryQueued = true;
    return this.success(null);
  }

  private search(payload: Record<string, unknown>): MockEnvelope {
    if (
      payload.bookId !== bookId ||
      typeof payload.query !== 'string' ||
      !this.committedPages.has(2)
    ) {
      return this.success([]);
    }
    return this.success([
      {
        snippet: committedText,
        locator: this.locator(2),
        sectionTitle: null,
      },
    ]);
  }

  private locator(page: number) {
    return {
      format: 'pdf',
      startPage: page,
      endPage: page,
      rectsByPage: null,
    };
  }

  private incrementClaim(pageNumber: number) {
    this.claimCounts.set(
      pageNumber,
      (this.claimCounts.get(pageNumber) ?? 0) + 1,
    );
  }

  private success(value: unknown): MockEnvelope {
    return { ok: true, value };
  }

  private failure(code: string): MockEnvelope {
    return {
      ok: false,
      error: {
        code,
        message: 'The synthetic local operation could not finish.',
        nextStep: 'Retry the deterministic local test operation.',
        diagnosticId: null,
      },
    };
  }
}

/* eslint-disable react-hooks/rules-of-hooks -- Playwright fixture callback. */
const test = base.extend<{ backend: SyntheticAiIndexBackend }>({
  backend: async ({ page }, use) => {
    const backend = new SyntheticAiIndexBackend(await makeTwoPageScannedPdf());
    await installMock(page, backend);
    await use(backend);
  },
});
/* eslint-enable react-hooks/rules-of-hooks */

test('J: rejecting the real ready-PDF entry stays local and authorizes nothing', async ({
  page,
  backend,
}) => {
  await page.goto('/library');
  await openAdvancedIndex(page);
  await expect(
    page.getByRole('dialog', { name: 'Start AI-assisted indexing?' }),
  ).toBeVisible();

  await page
    .getByRole('button', { name: 'Continue without AI indexing' })
    .click();
  await expect(page).toHaveURL(
    new RegExp(`/books/${bookId}/read\\?index=local-only$`),
  );
  await expect(page.getByText(exactLimitation, { exact: true })).toBeVisible();
  const renderedPages = page.locator('.pdf-viewer [data-page-number]');
  await expect(renderedPages).toHaveCount(2);
  await expect(renderedPages.first()).toBeVisible();
  await expect(page.getByText('无法打开教材。', { exact: true })).toHaveCount(
    0,
  );
  await expect(page.getByRole('button', { name: 'Search' })).toBeEnabled();
  await expect(
    page.getByRole('heading', {
      name: 'Self-made blank-page textbook',
      exact: true,
      level: 1,
    }),
  ).toBeVisible();

  const stats = backend.safeStats();
  expect(stats.calls.confirm_index_operation ?? 0).toBe(0);
  expect(stats.calls.create_index_run ?? 0).toBe(0);
  expect(stats.calls.update_provider_operation_consent ?? 0).toBe(0);
  expect(stats.providerDispatches).toBe(0);
  expect(stats.uploads).toBe(0);
  expect(stats.uploadedPageBatches).toEqual([]);
  expect(stats.remoteResources).toBe(0);
  expect(backend.externalOrigins).toEqual(new Set());
  await expectNoSeriousA11yIssues(page);
});

test('K: confirmed abnormal pages remain searchable after transient inputs disappear', async ({
  page,
  backend,
}) => {
  await startConfirmedRun(page);
  await expect(page.getByText(/Overall: Ready/u)).toBeVisible();
  await expect(page.getByText('No pages need review.')).toBeVisible();

  await page.goto(`/books/${bookId}/read`);
  await expect(
    page.getByRole('heading', {
      name: 'Self-made blank-page textbook',
      exact: true,
      level: 1,
    }),
  ).toBeVisible();
  const sourceReadsBeforeRemoval =
    backend.safeStats().calls.read_book_source ?? 0;
  backend.removeTransientDependencies();
  await page.getByRole('button', { name: 'Search' }).click();
  await page.getByLabel('Search book content').fill('durable optics');
  await page
    .locator('#reader-search-drawer')
    .getByRole('button', { name: 'Search', exact: true })
    .click();
  await expect(page.getByText(committedText)).toBeVisible();

  const stats = backend.safeStats();
  expect(stats.calls.confirm_index_operation).toBe(1);
  expect(stats.calls.create_index_run).toBe(1);
  expect(stats.calls.update_provider_operation_consent ?? 0).toBe(0);
  expect(stats.committedPages).toEqual([1, 2]);
  expect(stats.confirmedPages).toEqual([
    { pageNumber: 1, qualityReason: 'no_text' },
    { pageNumber: 2, qualityReason: 'no_text' },
  ]);
  expect(stats.claimCounts).toEqual({ 1: 1, 2: 1 });
  expect(stats.sourceReadsAfterRemoval).toBe(0);
  expect(stats.calls.read_book_source).toBe(sourceReadsBeforeRemoval);
  expect(stats.providerDispatches).toBe(1);
  expect(stats.uploads).toBe(1);
  expect(stats.uploadedPageBatches).toEqual([[1, 2]]);
  await assertBrowserPrivacy(page, backend);
});

test('L: partial failure retries only the stale-version-matched page', async ({
  page,
  backend,
}) => {
  backend.failFirstPageOnce = true;
  await startConfirmedRun(page);

  await expect(page.getByText(/Overall: Partial/u)).toBeVisible();
  await expect(page.getByText('Completed').locator('..')).toContainText('1');
  await expect(
    page
      .getByRole('region', { name: 'Index status' })
      .getByText('Failed', { exact: true })
      .locator('..'),
  ).toContainText('1');
  await expect(
    page.getByRole('button', { name: 'Page 1: failed' }),
  ).toBeVisible();

  await page.goto(`/books/${bookId}/read`);
  await expect(
    page.getByRole('heading', {
      name: 'Self-made blank-page textbook',
      exact: true,
      level: 1,
    }),
  ).toBeVisible();
  await expect(page.getByRole('button', { name: 'Search' })).toBeEnabled();
  await page.goto(`/books/${bookId}/index-quality/${runId}`);
  await page.getByRole('button', { name: 'Retry page' }).click();

  await expect(page.getByText(/Overall: Ready/u)).toBeVisible();
  await expect(page.getByText('No pages need review.')).toBeVisible();
  const stats = backend.safeStats();
  expect(stats.retryExpectedVersion).toBe(failedUpdatedAt);
  expect(stats.claimCounts).toEqual({ 1: 2, 2: 1 });
  expect(stats.committedPages).toEqual([1, 2]);
  expect(stats.calls.retry_index_page).toBe(1);
  await expectNoSeriousA11yIssues(page);
});

async function startConfirmedRun(page: Page) {
  await page.goto('/library');
  await openAdvancedIndex(page);
  await expect(
    page.getByRole('dialog', { name: 'Start AI-assisted indexing?' }),
  ).toContainText(
    'Profile: Loopback vision profile. Model: synthetic-vision-v1. Pages: 2.',
  );
  await expect(page.getByRole('dialog')).toContainText(
    'Local page images for these 2 abnormal pages will be sent for structured page analysis. This may incur provider charges.',
  );
  await page.getByRole('button', { name: 'Confirm and start' }).click();
  await expect(page).toHaveURL(
    new RegExp(`/books/${bookId}/index-quality/${runId}$`),
  );
}

async function openAdvancedIndex(page: Page) {
  await page.getByRole('button', { name: 'Prepare full-text Q&A' }).click();
  await expect(page).toHaveURL(new RegExp(`/books/${bookId}/index-start$`));
  await page
    .getByRole('button', {
      name: 'Advanced: full page-by-page visual index',
    })
    .click();
}

async function installMock(page: Page, backend: SyntheticAiIndexBackend) {
  page.on('console', (message) => backend.consoleMessages.push(message.text()));
  page.on('pageerror', (error) =>
    backend.consoleMessages.push(`pageerror:${error.message}`),
  );
  page.on('requestfailed', (request) =>
    backend.consoleMessages.push(
      `requestfailed:${new URL(request.url()).pathname}:${request.failure()?.errorText ?? 'unknown'}`,
    ),
  );
  page.on('response', (response) => {
    if (response.status() >= 400)
      backend.consoleMessages.push(
        `response:${new URL(response.url()).pathname}:${response.status()}`,
      );
  });
  page.on('request', (request) => {
    const origin = new URL(request.url()).origin;
    if (origin !== 'http://127.0.0.1:1420') backend.externalOrigins.add(origin);
  });
  await page.exposeFunction(
    '__p9BackendInvoke',
    (command: string, payload?: Record<string, unknown>) =>
      backend.invoke(command, payload),
  );
  await page.addInitScript(() => {
    type BinaryMarker = { __binary: number[] };
    type TestWindow = Window & {
      __p9BackendInvoke(
        command: string,
        payload?: Record<string, unknown>,
      ): Promise<MockEnvelope>;
      __TAURI_INTERNALS__: {
        invoke(
          command: string,
          payload?: Record<string, unknown> | Uint8Array,
          options?: { headers?: Record<string, string> },
        ): Promise<unknown>;
      };
    };
    const testWindow = window as TestWindow;
    testWindow.__TAURI_INTERNALS__ = {
      async invoke(command, payload = {}, options = {}) {
        let safePayload: Record<string, unknown>;
        if (command === 'submit_index_render_batch') {
          const body = payload as Uint8Array;
          const captures = JSON.parse(
            options.headers?.['x-textbooklens-index-captures'] ?? '[]',
          ) as Array<{ pageNumber?: unknown }>;
          safePayload = {
            captureCount: captures.length,
            encodedByteLength: body.byteLength,
            pageNumbers: captures.map((capture) => capture.pageNumber),
          };
          body.fill(0);
        } else {
          safePayload = payload as Record<string, unknown>;
        }
        const response = await testWindow.__p9BackendInvoke(
          command,
          safePayload,
        );
        if (!response.ok) throw response.error;
        const value = response.value as BinaryMarker | undefined;
        if (value && Array.isArray(value.__binary))
          return Uint8Array.from(value.__binary);
        return response.value;
      },
    };
  });
}

async function expectNoSeriousA11yIssues(page: Page) {
  const results = await new AxeBuilder({ page })
    .disableRules(['color-contrast'])
    .analyze();
  expect(
    results.violations.filter((violation) =>
      ['critical', 'serious'].includes(violation.impact ?? ''),
    ),
  ).toEqual([]);
}

async function assertBrowserPrivacy(
  page: Page,
  backend: SyntheticAiIndexBackend,
) {
  const visibleSurfaces = await page.evaluate(() =>
    JSON.stringify({
      body: document.body.textContent,
      local: Object.fromEntries(
        Array.from({ length: localStorage.length }, (_, index) => {
          const key = localStorage.key(index)!;
          return [key, localStorage.getItem(key)];
        }),
      ),
      session: Object.fromEntries(
        Array.from({ length: sessionStorage.length }, (_, index) => {
          const key = sessionStorage.key(index)!;
          return [key, sessionStorage.getItem(key)];
        }),
      ),
      history: history.state,
    }),
  );
  const logs = JSON.stringify(backend.consoleMessages);
  for (const sentinel of forbiddenBrowserSentinels) {
    expect(visibleSurfaces).not.toContain(sentinel);
    expect(logs).not.toContain(sentinel);
  }
  expect(backend.externalOrigins).toEqual(new Set());
}

async function makeTwoPageScannedPdf(): Promise<Uint8Array> {
  const fixture = await readFile('fixtures/source/scanned-textbook.pdf');
  const source = await PDFDocument.load(fixture, { updateMetadata: false });
  if (source.getPageCount() !== 1)
    throw new Error('synthetic scanned fixture must contain exactly one page');
  const document = await PDFDocument.create({ updateMetadata: false });
  const pages = await document.copyPages(source, [0, 0]);
  for (const page of pages) document.addPage(page);
  const bytes = await document.save({
    addDefaultPage: false,
    updateFieldAppearances: false,
    useObjectStreams: false,
  });
  return Uint8Array.from(bytes);
}
