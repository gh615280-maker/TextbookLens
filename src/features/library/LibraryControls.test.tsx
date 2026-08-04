import { cleanup, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { LanguageContext } from '../../app/LanguageProvider';
import { formatMessage } from '../../lib/i18n';
import { LibraryCommandBar } from './LibraryCommandBar';
import { LibrarySidebar } from './LibrarySidebar';

function renderControls() {
  const onFilterChange = vi.fn();
  const onQueryChange = vi.fn();
  const onSortChange = vi.fn();
  const onViewChange = vi.fn();
  render(
    <LanguageContext.Provider
      value={{
        uiLanguage: 'en',
        isLoading: false,
        statusMessage: null,
        switchLanguage: vi.fn(),
        message: (key, values) => formatMessage('en', key, values),
      }}
    >
      <LibrarySidebar filter="all" onFilterChange={onFilterChange} />
      <LibraryCommandBar
        filter="all"
        importDisabled={false}
        query=""
        sort="title"
        view="large"
        onImport={vi.fn(async () => {})}
        onQueryChange={onQueryChange}
        onSortChange={onSortChange}
        onViewChange={onViewChange}
      />
    </LanguageContext.Provider>,
  );
  return { onFilterChange, onQueryChange, onSortChange, onViewChange };
}

afterEach(cleanup);

describe('library controls', () => {
  it('limits the sidebar to the three specified filters and exposes semantic commands', async () => {
    const user = userEvent.setup();
    const { onFilterChange, onQueryChange, onSortChange, onViewChange } =
      renderControls();

    const sidebar = screen.getByRole('navigation', { name: 'Library folders' });
    expect(
      Array.from(sidebar.querySelectorAll('button')).map(
        (button) => button.textContent,
      ),
    ).toEqual(['All books', 'Recently opened', 'Processing']);
    await user.click(screen.getByRole('button', { name: 'Recently opened' }));
    expect(onFilterChange).toHaveBeenCalledWith('recent');

    expect(screen.getByText('Showing: All books')).toBeVisible();
    await user.type(
      screen.getByRole('searchbox', { name: 'Search books' }),
      'a',
    );
    expect(onQueryChange).toHaveBeenLastCalledWith('a');
    await user.selectOptions(
      screen.getByRole('combobox', { name: 'Sort books' }),
      'lastOpened',
    );
    expect(onSortChange).toHaveBeenCalledWith('lastOpened');
    await user.click(screen.getByRole('button', { name: 'Details' }));
    expect(onViewChange).toHaveBeenCalledWith('details');
  });
});
