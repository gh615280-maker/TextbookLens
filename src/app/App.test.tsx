import { afterEach, describe, expect, it } from 'vitest';
import { cleanup, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';

import { App } from './App';

describe('App', () => {
  afterEach(cleanup);

  it('renders the final navigation and marks the current route', async () => {
    render(<App initialEntries={['/library']} />);

    expect(await screen.findByRole('heading', { name: '书库' })).toBeVisible();
    expect(screen.getByRole('main')).toBeVisible();
    expect(screen.getByRole('link', { name: '书库' })).toHaveAttribute(
      'aria-current',
      'page',
    );
    expect(screen.getByRole('link', { name: '教学指令' })).toHaveAttribute(
      'href',
      '/teaching-instructions',
    );
    expect(screen.getByRole('link', { name: 'AI 服务' })).toHaveAttribute(
      'href',
      '/ai-services',
    );
    expect(screen.getByRole('link', { name: '设置' })).toHaveAttribute(
      'href',
      '/settings',
    );
  });

  it('uses the simplified onboarding shell without the main navigation', async () => {
    render(<App initialEntries={['/onboarding']} />);

    expect(
      await screen.findByRole('heading', { name: '开始使用' }),
    ).toBeVisible();
    expect(screen.queryByRole('navigation')).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: '应用语言' })).toBeVisible();
  });

  it('keeps the reader route outside the standard navigation', async () => {
    render(<App initialEntries={['/books/book-1/read']} />);

    expect(screen.queryByRole('navigation')).not.toBeInTheDocument();
    expect(await screen.findByRole('button', { name: '目录' })).toBeVisible();
  });

  it('moves focus to the main landmark through the skip link', async () => {
    const user = userEvent.setup();
    render(<App initialEntries={['/library']} />);

    await user.click(await screen.findByRole('link', { name: '跳到主要内容' }));

    expect(screen.getByRole('main')).toHaveFocus();
  });
});
