import { expect, test, type Frame, type Page } from '@playwright/test';
import { readFileSync } from 'node:fs';
import JSZip from 'jszip';

const BOOK = '31000000-0000-4000-8000-000000000001';
const SECTION = '31000000-0000-4000-8000-000000000002';
const PROFILE = '31000000-0000-4000-8000-000000000003';

async function archive() {
  const zip = new JSZip();
  zip.file('mimetype', 'application/epub+zip');
  zip.file(
    'META-INF/container.xml',
    '<?xml version="1.0"?><container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OPS/package.opf" media-type="application/oebps-package+xml"/></rootfiles></container>',
  );
  const items = Array.from(
    { length: 6 },
    (_, i) =>
      `<item id="s${i}" href="Text/s${i}.xhtml" media-type="application/xhtml+xml"/>`,
  ).join('');
  const spine = Array.from(
    { length: 6 },
    (_, i) => `<itemref idref="s${i}"/>`,
  ).join('');
  zip.file(
    'OPS/package.opf',
    `<?xml version="1.0"?><package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="uid"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:identifier id="uid">synthetic-epub</dc:identifier><dc:title>Synthetic EPUB</dc:title><dc:language>en</dc:language></metadata><manifest>${items}<item id="image" href="Images/diagram.png" media-type="image/png"/><item id="style" href="Styles/book.css" media-type="text/css"/></manifest><spine>${spine}</spine></package>`,
  );
  zip.file(
    'OPS/Images/diagram.png',
    readFileSync('fixtures/source/figure-energy.png'),
  );
  zip.file(
    'OPS/Styles/book.css',
    'body { margin: 0; padding: 20px; } img { width: 64px; height: 40px; } #target { color: rgb(23, 45, 67); }',
  );
  for (let i = 0; i < 6; i++) {
    const body =
      i === 0
        ? '<img id="cover" alt="Synthetic cover" src="../Images/diagram.png"/>'
        : i === 5
          ? '<h1>Synthetic chapter</h1><p>First eigenvalue occurrence.</p><p id="target">Second eigenvalue occurrence.</p><img id="figure" alt="Synthetic figure" src="../Images/diagram.png"/><img src="https://outside.invalid/track"/>' +
            '<p>Local archived text only.</p>'.repeat(30)
          : '';
    zip.file(
      `OPS/Text/s${i}.xhtml`,
      `<?xml version="1.0"?><html xmlns="http://www.w3.org/1999/xhtml"><head><title>Synthetic ${i}</title><link rel="stylesheet" href="../Styles/book.css"/></head><body>${body}</body></html>`,
    );
  }
  return [...(await zip.generateAsync({ type: 'uint8array' }))];
}

async function installBackend(page: Page) {
  await page.addInitScript(
    ({ bytes, bookId, sectionId, profileId }) => {
      const state = {
        saved: [] as Array<Record<string, unknown>>,
        unknown: [] as string[],
      };
      const settings = {
        onboardingCompleted: true,
        activeProviderProfileId: profileId,
        defaultLearningProfileId: profileId,
        defaultVisionProfileId: null,
        theme: 'system',
        contextMode: 'standard',
        uiLanguage: 'zh-CN',
        uiLanguageInitialized: true,
        firstReaderHintCompleted: true,
      };
      const book = {
        id: bookId,
        title: 'Synthetic EPUB',
        originalFilename: 'synthetic.epub',
        author: null,
        language: 'en',
        format: 'epub',
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
      const testWindow = window as unknown as {
        __epubReaderTest: typeof state;
        __TAURI_INTERNALS__: {
          invoke(
            command: string,
            payload?: Record<string, unknown>,
          ): Promise<unknown>;
        };
      };
      testWindow.__epubReaderTest = state;
      testWindow.__TAURI_INTERNALS__ = {
        async invoke(command, payload = {}) {
          switch (command) {
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
              return {
                book,
                lastLocator: JSON.parse(
                  sessionStorage.getItem('epub-reader-progress') ?? 'null',
                ),
              };
            case 'read_book_source':
              return new Uint8Array(bytes);
            case 'list_reader_sections':
              return [
                {
                  id: sectionId,
                  parentId: null,
                  ordinal: 0,
                  title: 'Synthetic chapter',
                  locator: {
                    format: 'epub',
                    cfi: 'epubcfi(/6/12!/4/2/1:0)',
                    sectionId,
                  },
                },
              ];
            case 'list_provider_profiles':
              return payload.operation === 'vision_learning'
                ? []
                : [
                    {
                      id: profileId,
                      kind: 'ollama',
                      displayName: 'Synthetic local model',
                      modelId: 'synthetic-local',
                      contextWindowTokens: 8192,
                      isActive: true,
                      credentialStatus: 'not_required',
                      validatedAt: '2026-09-08T00:00:00Z',
                    },
                  ];
            case 'list_annotation_markers':
            case 'search_book':
            case 'list_books':
              return [];
            case 'claim_index_render_batch':
              return { claims: [] };
            case 'save_reading_progress': {
              const locator = payload.locator as Record<string, unknown>;
              if (locator.sectionId !== sectionId)
                throw { code: 'INVALID_INPUT' };
              state.saved.push(locator);
              sessionStorage.setItem(
                'epub-reader-progress',
                JSON.stringify(locator),
              );
              return;
            }
            default:
              state.unknown.push(command);
              throw new Error('Unexpected test IPC: ' + command);
          }
        },
      };
    },
    {
      bytes: await archive(),
      bookId: BOOK,
      sectionId: SECTION,
      profileId: PROFILE,
    },
  );
}

async function frameWith(
  page: Page,
  selector: string,
): Promise<Frame | undefined> {
  for (const frame of page.frames())
    if (frame !== page.mainFrame() && (await frame.locator(selector).count()))
      return frame;
  return undefined;
}

test('real EPUB archive restores images, maps skipped spine entries, selects text, and restores a canonical reading location', async ({
  page,
}) => {
  const external: string[] = [];
  page.on('request', (request) => {
    if (
      /^https?:/.test(request.url()) &&
      !request.url().startsWith('http://127.0.0.1:1420/')
    )
      external.push(request.url());
  });
  await installBackend(page);
  const csp = JSON.parse(readFileSync('src-tauri/tauri.conf.json', 'utf8')).app
    .security.csp as string;
  await page.route(`**/books/${BOOK}/read`, async (route) => {
    const response = await route.fetch();
    await route.fulfill({
      response,
      headers: { ...response.headers(), 'content-security-policy': csp },
    });
  });
  await page.goto(`/books/${BOOK}/read`);
  await expect
    .poll(async () => Boolean(await frameWith(page, '#cover')))
    .toBe(true);
  const cover = (await frameWith(page, '#cover'))!;
  await expect
    .poll(() =>
      cover
        .locator('#cover')
        .evaluate(
          (image: HTMLImageElement) => image.complete && image.naturalWidth > 0,
        ),
    )
    .toBe(true);
  await expect(cover.locator('#cover')).toHaveCSS('width', '64px');
  await page.getByRole('button', { name: '目录', exact: true }).click();
  await page
    .getByRole('button', { name: 'Synthetic chapter', exact: true })
    .click();
  await expect
    .poll(async () => Boolean(await frameWith(page, '#target')))
    .toBe(true);
  const chapter = (await frameWith(page, '#target'))!;
  await expect(chapter.locator('#target')).toHaveCSS(
    'color',
    'rgb(23, 45, 67)',
  );
  await expect
    .poll(() =>
      chapter
        .locator('#figure')
        .evaluate(
          (image: HTMLImageElement) => image.complete && image.naturalWidth > 0,
        ),
    )
    .toBe(true);
  await chapter.locator('#target').dblclick({ position: { x: 84, y: 8 } });
  const menu = page.getByRole('menuitem', { name: '提问', exact: true });
  await expect(menu).toBeVisible();
  const wordBounds = (await chapter.locator('#target').boundingBox())!;
  const menuBounds = (await menu.boundingBox())!;
  expect(menuBounds.y).toBeGreaterThan(wordBounds.y);
  expect(menuBounds.y - wordBounds.y).toBeLessThan(180);
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          JSON.parse(sessionStorage.getItem('epub-reader-progress') ?? 'null')
            ?.sectionId,
      ),
    )
    .toBe(SECTION);
  expect(external).toEqual([]);
  await page.reload();
  await expect
    .poll(async () => Boolean(await frameWith(page, '#target')))
    .toBe(true);
  await expect(
    (await frameWith(page, '#target'))!.locator('#target'),
  ).toBeVisible();
  expect(
    await page.evaluate(
      () =>
        (window as unknown as { __epubReaderTest: { unknown: string[] } })
          .__epubReaderTest.unknown,
    ),
  ).toEqual([]);
  expect(external).toEqual([]);
});
