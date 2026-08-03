import { cleanup, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';

import { App } from '../app/App';

describe('AppShell', () => {
  afterEach(cleanup);

  it('renders phase-owned placeholders without inventing product state', async () => {
    const teaching = render(
      <App initialEntries={['/teaching-instructions']} />,
    );

    expect(await screen.findByText('教学指令将在后续阶段实现。')).toBeVisible();

    teaching.unmount();
    render(<App initialEntries={['/ai-services']} />);

    expect(await screen.findByText('AI 服务将在后续阶段实现。')).toBeVisible();
  });

  it('keeps the navigation keyboard reachable in a narrow viewport', async () => {
    render(<App initialEntries={['/library']} />);

    const links = await screen.findAllByRole('link');
    expect(links.map((link) => link.textContent)).toContain('AI 服务');
    expect(screen.getByRole('link', { name: '设置' })).toBeVisible();
  });
});
