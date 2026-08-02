import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { App } from './App';

describe('App', () => {
  it('renders Chinese library navigation with a visible main landmark', async () => {
    render(<App initialEntries={['/library']} />);

    expect(await screen.findByRole('heading', { name: '书库' })).toBeVisible();
    expect(screen.getByRole('main')).toBeVisible();
    expect(screen.getByRole('link', { name: '设置' })).toHaveAttribute(
      'href',
      '/settings',
    );
  });
});
