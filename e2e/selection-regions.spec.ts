import AxeBuilder from '@axe-core/playwright';
import { expect, test, type Page } from '@playwright/test';
import { readFile } from 'node:fs/promises';

const BOOKS = {
  docx: '11111111-1111-4111-8111-111111111111',
  pdf: '22222222-2222-4222-8222-222222222222',
  epub: '33333333-3333-4333-8333-333333333333',
} as const;
const SECTIONS = {
  docx: '44444444-4444-4444-8444-444444444444',
  pdf: '55555555-5555-4555-8555-555555555555',
  epub: '66666666-6666-4666-8666-666666666666',
} as const;
const BLOCK_ID = '77777777-7777-4777-8777-777777777777';
const PROFILE_ID = '88888888-8888-4888-8888-888888888888';
const TEXTBOOK_SENTINEL = 'P11_TEXTBOOK_BODY_SENTINEL';
const NOTE_SENTINEL = 'P11_PRIVATE_NOTE_SENTINEL';
const IMAGE_SENTINEL = 'P11_IMAGE_BYTES_SENTINEL';
const CREDENTIAL_SENTINEL = 'P11_CREDENTIAL_SENTINEL';
const PROMPT_SENTINEL = 'P11_PROMPT_SENTINEL';
const HASH_SENTINEL = 'a'.repeat(64);

type Format = keyof typeof BOOKS;
type SafePreparedRecord = {
  action: string;
  contentKind: string;
  anchorKind: string;
  selectionMatchesAnchor: boolean;
};
type SafeStartedRecord = {
  action: string;
  preparationId: string;
  requestId: string;
  summary: Record<string, unknown>;
};
type SafeCaptureRecord = {
  readonly width: number;
  readonly height: number;
  readonly encodedByteLength: number;
  readonly bluePixelRatio: number;
};

class SyntheticSelectionBackend {
  readonly calls = new Map<string, number>();
  readonly preparations: SafePreparedRecord[] = [];
  readonly started: SafeStartedRecord[] = [];
  readonly consoleErrors: string[] = [];
  readonly externalOrigins = new Set<string>();
  readonly pdf: number[];
  readonly scannedPdf: number[];
  readonly epub: number[];
  readonly bluePng: string;
  language: 'zh-CN' | 'zh-TW' | 'en' = 'en';
  pdfZoom = 1;
  notes: Array<Record<string, unknown>> = [];
  nextNote = 1;
  authorizations: string[] = [];
  stageCount = 0;
  discardCount = 0;
  providerStreams = 0;
  messageWrites = 0;
  useVisualPdf = false;
  stagedCapture: SafeCaptureRecord | null = null;

  constructor(
    pdf: Uint8Array,
    scannedPdf: Uint8Array,
    epub: Uint8Array,
    bluePng: Uint8Array,
  ) {
    this.pdf = [...pdf];
    this.scannedPdf = [...scannedPdf];
    this.epub = [...epub];
    this.bluePng = Buffer.from(bluePng).toString('base64');
  }

  invoke(command: string, payload: Record<string, unknown> = {}) {
    this.calls.set(command, (this.calls.get(command) ?? 0) + 1);
    switch (command) {
      case 'get_app_settings':
      case 'initialize_ui_language':
        return this.settings();
      case 'update_ui_language':
        this.language = payload.language as typeof this.language;
        return this.settings();
      case 'complete_first_reader_hint':
        return { ...this.settings(), firstReaderHintCompleted: true };
      case 'get_reader_settings':
        return {
          fontScale: 1,
          lineHeight: 1.6,
          readerWidth: 72,
          pdfZoom: this.pdfZoom,
          theme: 'system',
        };
      case 'get_reader_bootstrap':
        return this.bootstrap(this.formatForBook(String(payload.bookId)));
      case 'list_reader_sections':
        return [this.section(this.formatForBook(String(payload.bookId)))];
      case 'read_book_source': {
        const format = this.formatForBook(String(payload.bookId));
        return {
          __binary:
            format === 'pdf'
              ? this.useVisualPdf
                ? this.scannedPdf
                : this.pdf
              : this.epub,
        };
      }
      case 'read_derived_text':
        return this.docxHtml();
      case 'list_provider_profiles':
        return [
          {
            id: PROFILE_ID,
            kind: 'openai',
            displayName: 'Synthetic Profile',
            modelId: 'synthetic-model',
            contextWindowTokens: 32_000,
            isActive: true,
            credentialStatus: 'available',
            validatedAt: '2026-08-05T00:00:00.000Z',
          },
        ];
      case 'list_annotation_markers':
        return this.notes
          .filter((note) => note.bookId === payload.bookId)
          .map((note) => ({
            id: note.id,
            kind: 'note',
            accessibilityLabel: 'Open personal note',
            anchor: note.anchor,
            relocationStatus: 'primary',
          }));
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
        return undefined;
      case 'authorize_learning_request':
        this.authorizations.push(String(payload.decision));
        return payload.decision === 'allow'
          ? '99999999-9999-4999-8999-999999999999'
          : null;
      case 'stage_region_capture':
        this.stageCount += 1;
        this.stagedCapture = payload.capture as SafeCaptureRecord;
        return undefined;
      case 'discard_learning_preparation':
        this.discardCount += 1;
        return undefined;
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
      case 'save_reading_progress':
        return undefined;
      default:
        throw new Error(`Unexpected IPC command: ${command}`);
    }
  }

  safeStats() {
    return {
      calls: Object.fromEntries(this.calls),
      preparations: this.preparations,
      started: this.started,
      noteCount: this.notes.length,
      noteRevisions: this.notes.map((note) => note.revision),
      authorizations: this.authorizations,
      stageCount: this.stageCount,
      discardCount: this.discardCount,
      providerStreams: this.providerStreams,
      messageWrites: this.messageWrites,
      stagedCapture: this.stagedCapture,
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
      uiLanguage: this.language,
      uiLanguageInitialized: true,
      firstReaderHintCompleted: true,
    };
  }

  private formatForBook(bookId: string): Format {
    const entry = Object.entries(BOOKS).find(([, id]) => id === bookId);
    if (!entry) throw new Error('unknown synthetic book');
    return entry[0] as Format;
  }

  private bootstrap(format: Format) {
    return {
      book: {
        id: BOOKS[format],
        title: `Synthetic ${format.toUpperCase()} textbook`,
        originalFilename: `synthetic.${format}`,
        author: null,
        language: 'en',
        format,
        importStatus: 'ready',
        importErrorCode: null,
        importErrorMessage: null,
        importErrorStage: null,
        readingProgress: 0,
        createdAt: '2026-08-05T00:00:00.000Z',
        updatedAt: '2026-08-05T00:00:00.000Z',
        lastOpenedAt: null,
      },
      lastLocator: null,
    };
  }

  private section(format: Format) {
    const locator =
      format === 'pdf'
        ? { format: 'pdf', startPage: 1, endPage: 1, rectsByPage: null }
        : format === 'epub'
          ? {
              format: 'epub',
              cfi: 'epubcfi(/6/2!/4/2)',
              sectionId: SECTIONS.epub,
            }
          : {
              format: 'docx',
              startBlockId: BLOCK_ID,
              startOffset: 0,
              endBlockId: BLOCK_ID,
              endOffset: 0,
            };
    return {
      id: SECTIONS[format],
      parentId: null,
      ordinal: 0,
      title: `Synthetic ${format} section`,
      locator,
    };
  }

  private docxHtml() {
    return `<p data-section-id="${SECTIONS.docx}" data-block-id="${BLOCK_ID}">${TEXTBOOK_SENTINEL} ordinary reliable text for immutable selection and local notes.</p><figure data-section-id="${SECTIONS.docx}" data-block-id="aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"><img alt="Synthetic chart" width="160" height="120" style="width:160px;height:120px" src="data:image/png;base64,${this.bluePng}"></figure>`;
  }

  private prepare(metadata: Record<string, unknown>) {
    const anchor = metadata.anchor as Record<string, unknown>;
    const selectedText = metadata.selectedText;
    const exact =
      anchor.kind === 'text'
        ? ((
            (anchor.selection as Record<string, unknown>).quote as Record<
              string,
              unknown
            >
          ).exact as string)
        : (((
            (anchor.region as Record<string, unknown>).textFallback as Record<
              string,
              unknown
            > | null
          )?.exact as string | undefined) ?? null);
    this.preparations.push({
      action: String(metadata.action),
      contentKind: String(metadata.contentKind),
      anchorKind: String(anchor.kind),
      selectionMatchesAnchor: selectedText === exact,
    });
    const visual = metadata.contentKind === 'visual_region';
    return {
      preparationId: `${String(this.preparations.length).padStart(8, '0')}-aaaa-4aaa-8aaa-aaaaaaaaaaaa`,
      providerDisplayName: 'Synthetic Provider',
      profileDisplayName: 'Synthetic Profile',
      modelDisplayName: 'synthetic-model',
      estimatedInputTokens: 42,
      sourceCount: 2,
      citationCount: 2,
      omittedSourceCount: 0,
      willSendImage: visual,
      riskFlags: visual ? ['image_send', 'cost_risk'] : [],
      requiresBlockingConfirmation: visual,
      expiresAt: '2026-08-05T01:00:00.000Z',
      actionCategory: metadata.action,
    };
  }

  private start(preparationId: string) {
    const index = Number(preparationId.slice(0, 8));
    const prepared = this.preparations[index - 1];
    if (!prepared) throw { code: 'NOT_FOUND' };
    const requestId = `${String(index).padStart(8, '0')}-bbbb-4bbb-8bbb-bbbbbbbbbbbb`;
    const summary = {
      estimatedInputTokens: 42,
      sourceCount: 2,
      citationCount: 2,
      willSendImage: prepared.contentKind === 'visual_region',
      requiresBlockingConfirmation: prepared.contentKind === 'visual_region',
    };
    this.started.push({
      action: prepared.action,
      preparationId,
      requestId,
      summary,
    });
    return this.requestSnapshot(requestId);
  }

  private subscribe(requestId: string, afterSeq: number) {
    if (
      afterSeq !== 1 ||
      !this.started.some((item) => item.requestId === requestId)
    ) {
      throw { code: 'INVALID_INPUT' };
    }
    return this.requestSnapshot(requestId);
  }

  private requestSnapshot(requestId: string) {
    return {
      requestId,
      conversationId: null,
      status: 'preparing',
      text: '',
      usage: null,
      safeError: null,
      lastSeq: 1,
    };
  }

  private createNote(payload: Record<string, unknown>) {
    const note = {
      id: `bbbbbbbb-bbbb-4bbb-8bbb-${String(this.nextNote++).padStart(12, '0')}`,
      bookId: payload.bookId,
      sectionId: payload.sectionId,
      anchor: payload.anchor,
      selectedText: payload.selectedText,
      noteText: payload.noteText,
      revision: 1,
      createdAt: '2026-08-05T00:00:00.000Z',
      updatedAt: '2026-08-05T00:00:00.000Z',
    };
    this.notes.push(note);
    return note;
  }

  private getNote(payload: Record<string, unknown>) {
    const note = this.notes.find(
      (candidate) =>
        candidate.id === payload.noteId && candidate.bookId === payload.bookId,
    );
    if (!note) throw { code: 'NOT_FOUND' };
    return note;
  }

  private updateNote(payload: Record<string, unknown>) {
    const note = this.getNote(payload);
    if (note.revision !== payload.expectedRevision) {
      throw { code: 'REQUEST_CONFLICT' };
    }
    note.noteText = payload.noteText;
    note.revision = Number(note.revision) + 1;
    note.updatedAt = '2026-08-05T00:01:00.000Z';
    return note;
  }

  private deleteNote(payload: Record<string, unknown>) {
    const note = this.getNote(payload);
    if (note.revision !== payload.expectedRevision) {
      throw { code: 'REQUEST_CONFLICT' };
    }
    this.notes = this.notes.filter((candidate) => candidate !== note);
    return undefined;
  }
}

/* eslint-disable react-hooks/rules-of-hooks -- Playwright fixture callback. */
const selectionTest = test.extend<{ backend: SyntheticSelectionBackend }>({
  backend: async ({ page }, use) => {
    const [pdf, scannedPdf, epub, bluePng] = await Promise.all([
      readFile('fixtures/textbook.pdf'),
      readFile('fixtures/source/scanned-textbook.pdf'),
      readFile('fixtures/textbook.epub'),
      readFile('fixtures/source/vision/tiny-blue.png'),
    ]);
    const backend = new SyntheticSelectionBackend(
      pdf,
      scannedPdf,
      epub,
      bluePng,
    );
    await installMock(page, backend);
    await use(backend);
  },
});
/* eslint-enable react-hooks/rules-of-hooks */

selectionTest(
  'PDF scroll viewport fills the content area while reserving resize gutters',
  async ({ page, backend }) => {
    backend.pdfZoom = 3;
    await page.addInitScript(() => {
      localStorage.setItem(
        'textbooklens.reader-viewport.v1',
        JSON.stringify({ width: 1100, height: 520 }),
      );
    });
    await page.goto(`/books/${BOOKS.pdf}/read`);
    await expect(
      page.locator('.pdf-viewer [data-page-number="1"]'),
    ).toBeVisible();

    const geometry = await page.evaluate(() => {
      const frame = document.querySelector('.reader-document-frame');
      const documentRegion = document.querySelector(
        '.reader-document.pdf-reader',
      );
      const viewport = document.querySelector('.pdf-viewer-container');
      const viewer = document.querySelector('.pdf-viewer');
      if (!frame || !documentRegion || !viewport || !viewer)
        throw new Error('PDF reader geometry is unavailable');
      const rect = (element: Element) => {
        const value = element.getBoundingClientRect();
        return {
          left: value.left,
          top: value.top,
          right: value.right,
          bottom: value.bottom,
        };
      };
      return {
        frame: rect(frame),
        documentRegion: rect(documentRegion),
        viewport: rect(viewport),
        documentZoom: getComputedStyle(documentRegion).zoom,
        cssContentZoom: getComputedStyle(viewer).zoom,
        nativeScaleFactor: Number.parseFloat(
          getComputedStyle(viewer).getPropertyValue('--scale-factor'),
        ),
      };
    });

    expect(geometry.documentRegion).toEqual({
      left: geometry.frame.left + 1,
      top: geometry.frame.top + 1,
      right: geometry.frame.right - 13,
      bottom: geometry.frame.bottom - 13,
    });
    expect(geometry.viewport).toEqual(geometry.documentRegion);
    expect(geometry.documentZoom).toBe('1');
    expect(geometry.cssContentZoom).toBe('1');
    expect(geometry.nativeScaleFactor).toBeGreaterThan(3);
  },
);

selectionTest(
  'D: immutable ordinary text prepares locally and notes survive edit/restart',
  async ({ page, backend }) => {
    selectionTest.setTimeout(90_000);
    await page.goto(`/books/${BOOKS.docx}/read`);
    await expect(
      page.getByText(TEXTBOOK_SENTINEL, { exact: false }),
    ).toBeVisible();

    await selectDocxText(page);
    await expect.poll(() => backend.consoleErrors).toEqual([]);
    await expect(
      page.getByRole('menu', { name: 'Learning actions' }),
    ).toBeVisible();
    await expect(page.getByRole('menuitem', { name: 'Explain' })).toBeFocused();
    await page.keyboard.press('ArrowDown');
    await expect(
      page.getByRole('menuitem', { name: 'Give an example' }),
    ).toBeFocused();
    await page.keyboard.press('Escape');
    await expect(
      page.getByRole('menu', { name: 'Learning actions' }),
    ).toHaveCount(0);

    for (const action of ['Explain', 'Give an example', 'Derive']) {
      await selectDocxText(page);
      await page.getByRole('menuitem', { name: action }).click();
      await expect
        .poll(() => backend.started.length)
        .toBeGreaterThanOrEqual(
          action === 'Explain' ? 1 : action === 'Give an example' ? 2 : 3,
        );
      await expect(page.getByRole('dialog')).toHaveCount(0);
      await hideNewestPanel(page);
    }

    await selectDocxText(page);
    await page.getByRole('menuitem', { name: 'Add note' }).click();
    await page
      .getByRole('textbox', { name: 'Personal note' })
      .fill(NOTE_SENTINEL);
    await page.getByRole('button', { name: 'Save note' }).click();
    const marker = page
      .locator('[data-annotation-id] [data-marker-shape="note"]')
      .first();
    await expect(marker).toBeVisible();
    await marker.click();
    const editor = page.getByRole('textbox', { name: 'Personal note' });
    await expect(editor).toHaveValue(NOTE_SENTINEL);
    await editor.fill('edited local note');
    await page.getByRole('button', { name: 'Save note' }).click();
    await expect(editor).toHaveCount(0);

    await page.reload();
    await expect(
      page.getByText(TEXTBOOK_SENTINEL, { exact: false }),
    ).toBeVisible();
    const restartedMarker = page
      .locator('[data-annotation-id] [data-marker-shape="note"]')
      .first();
    await expect(restartedMarker).toBeVisible();
    await restartedMarker.click();
    await expect(
      page.getByRole('textbox', { name: 'Personal note' }),
    ).toHaveValue('edited local note');

    await switchLanguage(page, '简体中文', '学习操作', '解释');
    await switchLanguage(page, '繁體中文', '學習操作', '解釋');
    await switchLanguage(page, 'English', 'Learning actions', 'Explain');

    const stats = backend.safeStats();
    expect(stats.preparations.slice(0, 3)).toEqual([
      expect.objectContaining({
        action: 'explain',
        contentKind: 'text_selection',
      }),
      expect.objectContaining({
        action: 'example',
        contentKind: 'text_selection',
      }),
      expect.objectContaining({
        action: 'derive',
        contentKind: 'text_selection',
      }),
    ]);
    expect(
      stats.preparations.every((item) => item.selectionMatchesAnchor),
    ).toBe(true);
    expect(stats.calls.create_note).toBe(1);
    expect(stats.calls.update_note).toBe(1);
    expect(stats.noteRevisions).toEqual([2]);
    expect(stats.providerStreams).toBe(0);
    expect(stats.messageWrites).toBe(0);
    await assertPrivacy(page, backend);
  },
);

selectionTest(
  'E: reliable region text stays image-free for PDF, EPUB, and DOCX',
  async ({ page, backend }) => {
    selectionTest.setTimeout(90_000);
    for (const format of ['pdf', 'epub', 'docx'] as const) {
      const sectionCalls = backend.calls.get('list_reader_sections') ?? 0;
      const profileCalls = backend.calls.get('list_provider_profiles') ?? 0;
      await page.goto(`/books/${BOOKS[format]}/read`);
      await expect(
        page.getByRole('heading', {
          name: `Synthetic ${format.toUpperCase()} textbook`,
          level: 1,
        }),
      ).toBeVisible();
      await expect
        .poll(() => backend.calls.get('list_reader_sections') ?? 0)
        .toBeGreaterThan(sectionCalls);
      await expect
        .poll(() => backend.calls.get('list_provider_profiles') ?? 0)
        .toBeGreaterThan(profileCalls);
      await expect(
        page.getByRole('button', { name: 'Select area' }),
      ).toBeEnabled();
      await selectReliableRegion(page, format);
      await expect(
        page.getByRole('menu', { name: 'Learning actions' }),
      ).toBeVisible();
      const preparationCount = backend.preparations.length;
      await page.getByRole('menuitem', { name: 'Explain' }).click();
      await expect(page.getByRole('dialog')).toHaveCount(0);
      await expect
        .poll(() => backend.preparations.length)
        .toBe(preparationCount + 1);
      expect(backend.preparations.at(-1)).toMatchObject({
        action: 'explain',
        contentKind: 'reliable_text_region',
        anchorKind: 'region',
        selectionMatchesAnchor: true,
      });
      const handoff = backend.started.at(-1);
      expect(handoff).toMatchObject({
        action: 'explain',
        summary: {
          estimatedInputTokens: 42,
          sourceCount: 2,
          citationCount: 2,
          willSendImage: false,
          requiresBlockingConfirmation: false,
        },
      });
      expect(JSON.stringify(handoff)).not.toContain(TEXTBOOK_SENTINEL);
      expect(JSON.stringify(handoff)).not.toContain(HASH_SENTINEL);
      await hideNewestPanel(page);
    }
    const stats = backend.safeStats();
    expect(stats.stageCount).toBe(0);
    expect(stats.authorizations).toEqual([]);
    expect(stats.providerStreams).toBe(0);
    expect(stats.messageWrites).toBe(0);
    await assertPrivacy(page, backend);
  },
);

selectionTest(
  'F: visual region cancel discards and confirm stages once without execution',
  async ({ page, backend }) => {
    await page.setViewportSize({ width: 390, height: 720 });
    await page.goto(`/books/${BOOKS.docx}/read`);
    await selectVisualDocxRegion(page);
    const dialog = page.getByRole('dialog', {
      name: 'Confirm learning request',
    });
    await page.getByRole('menuitem', { name: 'Explain' }).click();
    await expect(dialog).toContainText(
      'Provider: Synthetic Provider. Profile: Synthetic Profile. Model: synthetic-model. Estimated input: 42 tokens. Sources: 2. Citations: 2.',
    );
    await expect(dialog).toContainText(
      'This request will send one local image after authorization.',
    );
    await expect(dialog).toContainText(
      'This request may incur provider charges.',
    );
    await dialog.getByRole('button', { name: 'Cancel' }).click();
    await expect.poll(() => backend.discardCount).toBe(1);
    expect(backend.stageCount).toBe(0);
    expect(backend.started).toHaveLength(0);
    expect(backend.notes).toHaveLength(0);

    await selectVisualDocxRegion(page);
    await page.getByRole('menuitem', { name: 'Explain' }).click();
    await dialog
      .getByRole('button', { name: 'Authorize and continue' })
      .click();
    await expect.poll(() => backend.stageCount).toBe(1);
    await expect.poll(() => backend.started.length).toBe(1);
    await hideNewestPanel(page);
    expect(backend.authorizations).toEqual(['deny', 'allow']);
    expect(backend.notes).toHaveLength(0);
    expect(backend.providerStreams).toBe(0);
    expect(backend.messageWrites).toBe(0);

    const axe = await new AxeBuilder({ page })
      .disableRules(['color-contrast'])
      .analyze();
    expect(
      axe.violations.filter((violation) =>
        ['critical', 'serious'].includes(violation.impact ?? ''),
      ),
    ).toEqual([]);
    await assertPrivacy(page, backend);
  },
);

selectionTest(
  'PDF visual selection stages the actual blue crop at native zoom coordinates',
  async ({ page, backend }) => {
    selectionTest.setTimeout(60_000);
    backend.useVisualPdf = true;
    backend.pdfZoom = 1.75;
    await page.goto(`/books/${BOOKS.pdf}/read`);
    const canvas = page.locator('.pdf-viewer [data-page-number="1"] canvas');
    await expect(canvas).toBeVisible();
    await canvas.evaluate((element) => {
      const viewport = element.closest('.pdf-viewer-container');
      if (viewport instanceof HTMLElement)
        viewport.scrollTop = element.clientHeight * 0.72;
    });
    await page.getByRole('button', { name: 'Select area' }).click();
    await drag(page, canvas, 0.16, 0.827, 0.35, 0.886);
    await page.getByRole('menuitem', { name: 'Explain' }).click();
    await page
      .getByRole('dialog', { name: 'Confirm learning request' })
      .getByRole('button', { name: 'Authorize and continue' })
      .click();

    await expect.poll(() => backend.stagedCapture).not.toBeNull();
    expect(backend.stagedCapture).toEqual(
      expect.objectContaining({
        width: expect.any(Number),
        height: expect.any(Number),
        encodedByteLength: expect.any(Number),
      }),
    );
    expect(backend.stagedCapture!.width).toBeGreaterThan(100);
    expect(backend.stagedCapture!.height).toBeGreaterThan(40);
    expect(backend.stagedCapture!.encodedByteLength).toBeGreaterThan(100);
    expect(backend.stagedCapture!.bluePixelRatio).toBeGreaterThan(0.9);
    await expect.poll(() => backend.started.length).toBe(1);
    await assertPrivacy(page, backend);
  },
);

async function installMock(page: Page, backend: SyntheticSelectionBackend) {
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
    '__phase11Invoke',
    (command: string, payload?: Record<string, unknown>) =>
      backend.invoke(command, payload),
  );
  await page.addInitScript(() => {
    type Envelope = { __binary: number[] };
    type TestWindow = Window & {
      __phase11Invoke(
        command: string,
        payload?: Record<string, unknown>,
      ): Promise<unknown>;
      __TAURI_INTERNALS__: {
        transformCallback(callback: (payload: unknown) => void): number;
        unregisterCallback(id: number): void;
        invoke(
          command: string,
          payload?: Record<string, unknown> | Uint8Array,
          options?: { headers?: Record<string, string> },
        ): Promise<unknown>;
      };
    };
    const testWindow = window as TestWindow;
    let nextCallbackId = 1;
    testWindow.__TAURI_INTERNALS__ = {
      transformCallback() {
        return nextCallbackId++;
      },
      unregisterCallback() {},
      async invoke(command, payload = {}) {
        let safePayload: Record<string, unknown>;
        if (command === 'stage_region_capture') {
          const body = payload as Uint8Array;
          const bitmap = await createImageBitmap(
            new Blob([Uint8Array.from(body)], { type: 'image/png' }),
          );
          const canvas = document.createElement('canvas');
          canvas.width = bitmap.width;
          canvas.height = bitmap.height;
          const context = canvas.getContext('2d', { alpha: false });
          if (!context) throw new Error('synthetic capture inspection failed');
          context.drawImage(bitmap, 0, 0);
          bitmap.close();
          const pixels = context.getImageData(
            0,
            0,
            canvas.width,
            canvas.height,
          ).data;
          let bluePixels = 0;
          for (let index = 0; index < pixels.length; index += 4) {
            if (
              pixels[index] < 80 &&
              pixels[index + 1] < 140 &&
              pixels[index + 2] > 180
            )
              bluePixels += 1;
          }
          safePayload = {
            encodedByteLength: body.byteLength,
            capture: {
              width: canvas.width,
              height: canvas.height,
              encodedByteLength: body.byteLength,
              bluePixelRatio: bluePixels / (canvas.width * canvas.height),
            },
          };
          canvas.width = 0;
          canvas.height = 0;
          body.fill(0);
        } else if (command === 'subscribe_learning_request') {
          const args = payload as Record<string, unknown>;
          safePayload = {
            requestId: args.requestId,
            afterSeq: args.afterSeq,
          };
        } else {
          safePayload = payload as Record<string, unknown>;
        }
        const value = await testWindow.__phase11Invoke(command, safePayload);
        if (
          value &&
          typeof value === 'object' &&
          '__binary' in value &&
          Array.isArray((value as Envelope).__binary)
        ) {
          return Uint8Array.from((value as Envelope).__binary);
        }
        return value;
      },
    };
  });
}

async function selectDocxText(page: Page) {
  await page.evaluate(
    ({ blockId }) => {
      const block = document.querySelector(`[data-block-id="${blockId}"]`);
      const node = block?.firstChild;
      if (!(node instanceof Text))
        throw new Error('missing synthetic text node');
      const range = document.createRange();
      range.setStart(node, 0);
      range.setEnd(node, Math.min(node.data.length, 48));
      const selection = window.getSelection();
      selection?.removeAllRanges();
      selection?.addRange(range);
      block.dispatchEvent(new MouseEvent('mouseup', { bubbles: true }));
    },
    { blockId: BLOCK_ID },
  );
}

async function selectReliableRegion(page: Page, format: Format) {
  await page.getByRole('button', { name: 'Select area' }).click();
  if (format === 'docx') {
    await drag(page, page.locator(`[data-block-id="${BLOCK_ID}"]`));
    return;
  }
  if (format === 'pdf') {
    const pageBox = page.locator('.pdf-viewer [data-page-number="1"]');
    await expect(pageBox).toBeVisible();
    const bounds = await pageBox.boundingBox();
    if (!bounds) throw new Error('synthetic PDF page has no bounds');
    const start = {
      x: bounds.x + bounds.width * 0.08,
      y: bounds.y + bounds.height * 0.08,
    };
    const end = {
      x: bounds.x + bounds.width * 0.72,
      y: bounds.y + bounds.height * 0.35,
    };
    await pageBox.dispatchEvent('pointerdown', {
      button: 0,
      clientX: start.x,
      clientY: start.y,
      pointerId: 1,
      pointerType: 'mouse',
    });
    await page.evaluate(({ x, y }) => {
      window.dispatchEvent(
        new PointerEvent('pointerup', {
          bubbles: true,
          button: 0,
          clientX: x,
          clientY: y,
          pointerId: 1,
          pointerType: 'mouse',
        }),
      );
    }, end);
    return;
  }
  let paragraph: ReturnType<Page['locator']> | undefined;
  await expect
    .poll(async () => {
      for (const frame of page.frames()) {
        const candidate = frame.locator('p').filter({ hasText: /.+/u }).first();
        if (await candidate.isVisible()) {
          paragraph = candidate;
          return true;
        }
      }
      return false;
    })
    .toBe(true);
  if (!paragraph) throw new Error('synthetic EPUB paragraph is unavailable');
  const bounds = await paragraph.evaluate((element) => {
    const rect = element.getBoundingClientRect();
    return { x: rect.x, y: rect.y, width: rect.width, height: rect.height };
  });
  await paragraph.dispatchEvent('pointerdown', {
    button: 0,
    clientX: bounds.x + bounds.width * 0.1,
    clientY: bounds.y + bounds.height * 0.1,
    pointerId: 1,
    pointerType: 'mouse',
  });
  await paragraph.dispatchEvent('pointerup', {
    button: 0,
    clientX: bounds.x + bounds.width * 0.8,
    clientY: bounds.y + bounds.height * 0.8,
    pointerId: 1,
    pointerType: 'mouse',
  });
}

async function selectVisualDocxRegion(page: Page) {
  await page.getByRole('button', { name: 'Select area' }).click();
  const image = page.getByRole('img', { name: 'Synthetic chart' });
  await expect(image).toBeVisible();
  await drag(page, image, 0.1, 0.1, 0.8, 0.8);
  await expect(
    page.getByRole('menu', { name: 'Learning actions' }),
  ).toBeVisible();
}

async function drag(
  page: Page,
  locator: ReturnType<Page['locator']>,
  startX = 0.1,
  startY = 0.1,
  endX = 0.8,
  endY = 0.8,
) {
  const box = await locator.boundingBox();
  if (!box) throw new Error('synthetic region target has no bounds');
  await page.mouse.move(
    box.x + box.width * startX,
    box.y + box.height * startY,
  );
  await page.mouse.down();
  await page.mouse.move(box.x + box.width * endX, box.y + box.height * endY);
  await page.mouse.up();
}

async function switchLanguage(
  page: Page,
  language: string,
  menuName: string,
  explainName: string,
) {
  await page.goto('/settings');
  const languageButton = page.getByRole('button', {
    name: /Application language|应用语言|應用程式語言/u,
  });
  await languageButton.click();
  await page.getByRole('menuitemradio', { name: language }).click();
  await page.goto(`/books/${BOOKS.docx}/read`);
  await expect(
    page.getByText(TEXTBOOK_SENTINEL, { exact: false }),
  ).toBeVisible();
  await selectDocxText(page);
  await expect(page.getByRole('menu', { name: menuName })).toBeVisible();
  await expect(page.getByRole('menuitem', { name: explainName })).toBeVisible();
  await page.keyboard.press('Escape');
}

async function hideNewestPanel(page: Page) {
  const hide = page.getByRole('button', { name: 'Hide' }).last();
  await expect(hide).toBeVisible();
  await hide.click();
}

async function assertPrivacy(page: Page, backend: SyntheticSelectionBackend) {
  const surfaces = await page.evaluate(() =>
    JSON.stringify({
      localStorage: { ...localStorage },
      sessionStorage: { ...sessionStorage },
      url: location.href,
      history: history.state,
      datasets: [...document.querySelectorAll<HTMLElement>('*')]
        .map((element) => ({ ...element.dataset }))
        .filter((dataset) => Object.keys(dataset).length > 0),
    }),
  );
  const logs = JSON.stringify(backend.consoleErrors);
  for (const sentinel of [
    TEXTBOOK_SENTINEL,
    NOTE_SENTINEL,
    IMAGE_SENTINEL,
    CREDENTIAL_SENTINEL,
    PROMPT_SENTINEL,
    HASH_SENTINEL,
  ]) {
    expect(surfaces).not.toContain(sentinel);
    expect(logs).not.toContain(sentinel);
  }
  expect(backend.externalOrigins).toEqual(new Set());
  expect(
    backend.consoleErrors.filter(
      (message) =>
        !/^Blocked script execution in 'about:srcdoc' because the document's frame is sandboxed and the 'allow-scripts' permission is not set\.$/u.test(
          message,
        ),
    ),
  ).toEqual([]);
  expect(
    [...backend.calls.keys()].filter((command) =>
      /(stream|consume|message|conversation|provider_request)/u.test(command),
    ),
  ).toEqual([]);
}
