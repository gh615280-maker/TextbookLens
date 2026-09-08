import { expect, test, type Page } from '@playwright/test';
import { readFileSync } from 'node:fs';

test.use({ screenshot: 'off', trace: 'off', video: 'off' });

const BOOK = '42000000-0000-4000-8000-000000000001';
async function installBackend(page: Page, bytes: number[], pages: number[]) {
  await page.addInitScript(
    ({ bytes, pages, bookId }) => {
      const settings = {
        onboardingCompleted: true,
        activeProviderProfileId: null,
        defaultLearningProfileId: null,
        defaultVisionProfileId: null,
        theme: 'system',
        contextMode: 'standard',
        uiLanguage: 'zh-CN',
        uiLanguageInitialized: true,
        firstReaderHintCompleted: true,
      };
      const book = {
        id: bookId,
        title: '阅读验证教材',
        originalFilename: 'reader-check.pdf',
        author: null,
        language: 'zh',
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
        createdAt: '2026-09-08T00:00:00Z',
        updatedAt: '2026-09-08T00:00:00Z',
        lastOpenedAt: null,
      };
      (
        window as unknown as { __TAURI_INTERNALS__: unknown }
      ).__TAURI_INTERNALS__ = {
        async invoke(command: string) {
          switch (command) {
            case 'get_onboarding_state':
              return { canSkipOnboarding: true };
            case 'get_app_settings':
            case 'initialize_ui_language':
            case 'complete_first_reader_hint':
              return settings;
            case 'get_reader_settings':
              return {
                fontScale: 1,
                lineHeight: 1.6,
                readerWidth: 72,
                pdfZoom: 1,
                theme: 'system',
              };
            case 'get_reader_bootstrap':
              return { book, lastLocator: null };
            case 'read_book_source':
              return new Uint8Array(bytes);
            case 'list_reader_sections':
              return pages.map((p, i) => ({
                id: `42000000-0000-4000-8000-${String(i + 2).padStart(12, '0')}`,
                parentId: null,
                ordinal: i,
                title: `第 ${p} 页`,
                locator: {
                  format: 'pdf',
                  startPage: p,
                  endPage: p,
                  rectsByPage: null,
                },
              }));
            case 'list_provider_profiles':
            case 'list_annotation_markers':
            case 'search_book':
              return [];
            case 'list_books':
              return [
                book,
                {
                  ...book,
                  id: '42000000-0000-4000-8000-000000000005',
                  title:
                    '一本用于检验较长中文书名、格式标识和导入状态能否清晰排列的测试教材',
                  format: 'epub',
                },
                {
                  ...book,
                  id: '42000000-0000-4000-8000-000000000006',
                  title:
                    'A long synthetic textbook title for testing the library layout',
                  format: 'docx',
                },
              ];
            case 'claim_index_render_batch':
              return { claims: [] };
            case 'save_reading_progress':
              return;
            default:
              throw new Error('Unexpected test IPC: ' + command);
          }
        },
      };
    },
    { bytes, pages, bookId: BOOK },
  );
}

async function darkPixels(page: Page, number: number) {
  const canvas = page
    .locator(`.page[data-page-number="${number}"] canvas`)
    .first();
  if (!(await canvas.count())) return 0;
  return canvas.evaluate((canvas: HTMLCanvasElement) => {
    const context = canvas.getContext('2d');
    if (!context || !canvas.width || !canvas.height) return 0;
    const data = context.getImageData(0, 0, canvas.width, canvas.height).data;
    let count = 0;
    for (let i = 0; i < data.length; i += 64)
      if (
        data[i] < 150 &&
        data[i + 1] < 150 &&
        data[i + 2] < 150 &&
        data[i + 3] > 0
      )
        count++;
    return count;
  });
}

test('library cards separate titles and status, and detail columns align', async ({
  page,
}) => {
  await page.setViewportSize({ width: 850, height: 700 });
  await installBackend(
    page,
    [...readFileSync('fixtures/textbook.pdf')],
    [1, 2],
  );
  await page.goto('/library');
  await expect(page.locator('.library-card')).toHaveCount(3);
  await expect(page.locator('.library-grid')).toHaveCSS(
    'list-style-type',
    'none',
  );
  const card = page.locator('.library-card').nth(1);
  const title = await card.locator('.library-item__title').boundingBox();
  const status = await card
    .locator('.library-item__import-status')
    .boundingBox();
  expect(title!.y + title!.height).toBeLessThanOrEqual(status!.y);
  await page.getByRole('button', { name: '详细列表', exact: true }).click();
  const headers = page.locator(
    '.library-details__header [role="columnheader"]',
  );
  const cells = page
    .locator('.library-details__row')
    .first()
    .locator('[role="gridcell"]');
  for (let i = 0; i < 6; i++)
    expect(
      Math.abs(
        (await headers.nth(i).boundingBox())!.x -
          (await cells.nth(i).boundingBox())!.x,
      ),
    ).toBeLessThan(1);
});

test('PDF navigation preserves the toolbar and resize handles do not cover its scrollbar', async ({
  page,
}) => {
  await installBackend(
    page,
    [...readFileSync('fixtures/textbook.pdf')],
    [1, 2],
  );
  await page.goto(`/books/${BOOK}/read`);
  await expect.poll(() => darkPixels(page, 1)).toBeGreaterThan(50);
  await page.getByRole('button', { name: '目录', exact: true }).click();
  await page.getByRole('button', { name: '第 2 页', exact: true }).click();
  await expect.poll(() => darkPixels(page, 2)).toBeGreaterThan(50);
  const toolbar = await page
    .getByRole('toolbar', { name: '阅读工具栏' })
    .boundingBox();
  expect(toolbar!.y).toBeGreaterThanOrEqual(0);
  const content = await page.locator('.pdf-viewer-container').boundingBox();
  const handle = await page
    .getByRole('button', { name: '调整教材区域宽度', exact: true })
    .boundingBox();
  expect(content!.x + content!.width).toBeLessThanOrEqual(handle!.x);
});

test.describe('opt-in local scan decoder', () => {
  test('JBIG2 pages render with packaged decoders under the production CSP', async ({
    page,
  }) => {
    test.skip(
      !process.env.TEXTBOOKLENS_SCAN_PDF,
      'Requires a user-designated local scan PDF',
    );
    const external: string[] = [];
    page.on('request', (request) => {
      if (
        /^https?:/.test(request.url()) &&
        !request.url().startsWith('http://127.0.0.1:1420/')
      )
        external.push(request.url());
    });
    await installBackend(
      page,
      [...readFileSync(process.env.TEXTBOOKLENS_SCAN_PDF!)],
      [1, 10],
    );
    const csp = JSON.parse(readFileSync('src-tauri/tauri.conf.json', 'utf8'))
      .app.security.csp as string;
    await page.route(`**/books/${BOOK}/read`, async (route) => {
      const response = await route.fetch();
      await route.fulfill({
        response,
        headers: { ...response.headers(), 'content-security-policy': csp },
      });
    });
    const decoder = page.waitForResponse(
      (response) =>
        response.url().endsWith('/pdfjs/wasm/jbig2.wasm') && response.ok(),
    );
    await page.goto(`/books/${BOOK}/read`);
    await decoder;
    await expect
      .poll(() => darkPixels(page, 1), { timeout: 30000 })
      .toBeGreaterThan(100);
    await page.getByRole('button', { name: '目录', exact: true }).click();
    await page.getByRole('button', { name: '第 10 页', exact: true }).click();
    await expect
      .poll(() => darkPixels(page, 10), { timeout: 30000 })
      .toBeGreaterThan(100);
    expect(external).toEqual([]);
  });
});
