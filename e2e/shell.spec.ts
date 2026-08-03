import { expect, test } from '@playwright/test';

const routes = [
  '/onboarding',
  '/library',
  '/books/00000000-0000-4000-8000-000000000001/read',
  '/books/00000000-0000-4000-8000-000000000001/overview',
  '/settings',
];

test('shell routes render without external network requests', async ({
  page,
}) => {
  const externalRequests = new Set<string>();
  page.on('request', (request) => {
    const url = new URL(request.url());
    if (
      !['127.0.0.1', 'localhost'].includes(url.hostname) &&
      !['data:', 'blob:'].includes(url.protocol)
    ) {
      externalRequests.add(`${url.protocol}//${url.host}`);
    }
  });

  for (const route of routes) {
    await page.goto(route);
    await expect(page.getByRole('heading', { level: 1 })).toBeVisible();
  }
  expect([...externalRequests]).toEqual([]);
});

test('normal, onboarding, and reader routes use their distinct shells', async ({
  page,
}) => {
  await page.goto('/library');
  await expect(page.getByRole('navigation', { name: '主导航' })).toBeVisible();
  await expect(page.getByRole('button', { name: '应用语言' })).toBeVisible();
  await expect(page.getByRole('toolbar')).toHaveCount(0);

  await page.goto('/onboarding');
  await expect(page.getByRole('heading', { name: '开始使用' })).toBeVisible();
  await expect(page.getByRole('button', { name: '应用语言' })).toBeVisible();
  await expect(page.getByRole('navigation')).toHaveCount(0);

  await page.goto('/books/00000000-0000-4000-8000-000000000001/read');
  await expect(page.getByRole('toolbar', { name: '阅读工具栏' })).toBeVisible();
  await expect(page.getByRole('button', { name: '应用语言' })).toHaveCount(0);
  await expect(page.getByRole('navigation', { name: '主导航' })).toHaveCount(0);
});

test('keyboard users can skip navigation', async ({ page }) => {
  await page.goto('/library');
  await page.keyboard.press('Tab');
  const skipLink = page.getByRole('link', { name: '跳到主要内容' });
  await expect(skipLink).toBeFocused();
  await page.keyboard.press('Enter');
  await expect(page.locator('#main-content')).toBeFocused();
});
