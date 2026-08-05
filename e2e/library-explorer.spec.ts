import AxeBuilder from '@axe-core/playwright';
import { expect, test } from '@playwright/test';

const book = {
  id: '22222222-2222-4222-8222-222222222222',
  title: 'Explorer fixture',
  originalFilename: 'explorer.docx',
  author: 'Fixture author',
  language: 'en',
  format: 'docx',
  importStatus: 'ready',
  importErrorCode: null,
  importErrorMessage: null,
  importErrorStage: null,
  readingProgress: 0,
  indexAggregate: {
    status: 'needs_review',
    totalPages: 3,
    indexedPages: 2,
    reviewPages: 1,
    failedPages: 0,
  },
  createdAt: '2026-08-04T00:00:00.000Z',
  updatedAt: '2026-08-04T00:00:00.000Z',
  lastOpenedAt: null,
};

test.beforeEach(async ({ page }) => {
  await page.addInitScript(
    (books) => {
      const app = window as unknown as {
        __TAURI_INTERNALS__: { invoke(command: string): Promise<unknown> };
      };
      app.__TAURI_INTERNALS__ = {
        async invoke(command) {
          if (command === 'get_onboarding_state')
            return {
              step: 'ready',
              selectedBook: books[0],
              hasReadyBook: true,
              learningProfileConnected: true,
              visionProfileConnected: false,
              localTextQuality: 'ready',
              canSkipOnboarding: true,
            };
          if (command === 'list_books') return books;
          if (command === 'find_current_index_run_for_book')
            return {
              runId: '11111111-1111-4111-8111-111111111111',
              bookId: books[0].id,
              controlStatus: 'running',
              aggregateStatus: 'needs_review',
              pages: {
                total: 3,
                notRequired: 0,
                queued: 0,
                rendering: 0,
                sending: 0,
                parsing: 0,
                validating: 0,
                indexed: 2,
                needsReview: 1,
                failed: 0,
                cancelled: 0,
              },
              updatedAt: '2026-08-04T00:00:00.000Z',
            };
          throw new Error(`Unexpected IPC command: ${command}`);
        },
      };
    },
    [book],
  );
});

test('explorer has two views, search, keyboard menu actions, narrow layout, and axe coverage', async ({
  page,
}) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto('/library');
  const item = page.getByRole('button', {
    name: 'Explorer fixture',
    exact: true,
  });
  await expect(item).toBeVisible();
  await page.getByRole('searchbox', { name: '搜索教材' }).fill('fixture');
  await page.getByRole('button', { name: '详细列表' }).click();
  await expect(page.getByRole('grid')).toBeVisible();
  await page.getByRole('button', { name: '大图标' }).click();
  await item.focus();
  await page.keyboard.press('Shift+F10');
  await expect(page.getByRole('menu', { name: '教材操作' })).toBeVisible();
  await page.getByRole('menuitem', { name: '索引状态' }).click();
  await expect(page).toHaveURL(
    /index-quality\/11111111-1111-4111-8111-111111111111$/,
  );
  await page.goto('/library');
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
});
