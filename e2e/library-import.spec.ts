import AxeBuilder from '@axe-core/playwright';
import { expect, test } from '@playwright/test';

const readyBook = {
  id: '4f9a2c86-0da8-4dd4-a255-39b4cff89c66',
  title: '线性代数',
  originalFilename: 'linear-algebra.pdf',
  author: 'TextbookLens',
  language: 'zh-CN',
  format: 'pdf',
  importStatus: 'ready',
  importErrorCode: null,
  importErrorMessage: null,
  importErrorStage: null,
  readingProgress: 0.375,
  createdAt: '2026-08-01T00:00:00Z',
  updatedAt: '2026-08-01T18:30:45Z',
  lastOpenedAt: '2026-08-01T18:30:45Z',
};

const failedBook = {
  ...readyBook,
  id: '70c92c3b-d44a-5345-a7eb-839e79c5b322',
  title: '导入失败教材',
  originalFilename: 'damaged.epub',
  format: 'epub',
  importStatus: 'failed',
  importErrorCode: 'FILE_CORRUPTED',
  importErrorMessage: '文件已损坏或无法读取。',
  importErrorStage: 'parsing',
  readingProgress: 0,
  lastOpenedAt: null,
};

test.beforeEach(async ({ page }) => {
  await page.addInitScript(
    ({ books }) => {
      type IpcCall = { command: string; payload: unknown };
      const testWindow = window as unknown as {
        __TAURI_INTERNALS__: {
          invoke(command: string, payload: unknown): Promise<unknown>;
        };
        __e2eIpcCalls: IpcCall[];
      };
      testWindow.__e2eIpcCalls = [];
      testWindow.__TAURI_INTERNALS__ = {
        async invoke(command, payload) {
          testWindow.__e2eIpcCalls.push({ command, payload });
          if (command === 'list_books') return books;
          if (command === 'plugin:dialog|open') return null;
          if (command === 'delete_failed_import') return null;
          throw new Error(`Unexpected IPC command: ${command}`);
        },
      };
    },
    { books: [readyBook, failedBook] },
  );
});

test('library exposes ready/failed states and an exact import picker contract', async ({
  page,
}) => {
  await page.goto('/library');

  await expect(page.getByRole('heading', { name: '线性代数' })).toBeVisible();
  await expect(
    page.getByRole('button', { name: '打开《线性代数》' }),
  ).toBeEnabled();
  await expect(page.getByText('damaged.epub')).toBeVisible();
  await expect(page.getByText('解析失败')).toBeVisible();
  await expect(
    page.getByRole('button', { name: '重新选择并重试' }),
  ).toBeEnabled();
  await expect(
    page.getByRole('button', { name: '删除失败记录' }),
  ).toBeEnabled();

  await page.getByRole('button', { name: '导入教材' }).click();
  const dialogCall = await page.evaluate(() => {
    const calls = (
      window as unknown as {
        __e2eIpcCalls: Array<{ command: string; payload: unknown }>;
      }
    ).__e2eIpcCalls;
    return calls.find((call) => call.command === 'plugin:dialog|open');
  });
  expect(dialogCall).toEqual({
    command: 'plugin:dialog|open',
    payload: {
      options: {
        multiple: false,
        directory: false,
        filters: [{ name: '教材', extensions: ['pdf', 'epub', 'docx'] }],
      },
    },
  });
  await expect(page).toHaveURL(/\/library$/);
});

test('library import surface has no automatically detectable accessibility violations', async ({
  page,
}) => {
  await page.goto('/library');
  await expect(page.getByRole('heading', { name: '线性代数' })).toBeVisible();

  const results = await new AxeBuilder({ page }).analyze();
  expect(results.violations).toEqual([]);
});
