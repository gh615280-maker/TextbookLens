import { useMessage } from '../../app/LanguageProvider';
import { ImportButton } from '../import/ImportButton';

import type { LibraryFilter, LibrarySort, LibraryView } from './library-state';

interface LibraryCommandBarProps {
  filter: LibraryFilter;
  importDisabled: boolean;
  query: string;
  sort: LibrarySort;
  view: LibraryView;
  onImport(sourcePath: string): Promise<void>;
  onQueryChange(query: string): void;
  onSortChange(sort: LibrarySort): void;
  onViewChange(view: LibraryView): void;
}

export function LibraryCommandBar({
  filter,
  importDisabled,
  query,
  sort,
  view,
  onImport,
  onQueryChange,
  onSortChange,
  onViewChange,
}: LibraryCommandBarProps) {
  const message = useMessage();

  return (
    <div
      aria-label={message('library.commandBarLabel')}
      className="library-command-bar"
      role="toolbar"
      style={{ display: 'flex', flexWrap: 'wrap', gap: 'var(--space-3)' }}
    >
      <ImportButton disabled={importDisabled} onSelect={onImport} />
      <label>
        <span>{message('library.searchLabel')}</span>
        <input
          type="search"
          value={query}
          onChange={(event) => onQueryChange(event.target.value)}
        />
      </label>
      <label>
        <span>{message('library.sortLabel')}</span>
        <select
          value={sort}
          onChange={(event) => onSortChange(event.target.value as LibrarySort)}
        >
          <option value="title">{message('library.sort.title')}</option>
          <option value="lastOpened">
            {message('library.sort.lastOpened')}
          </option>
          <option value="imported">{message('library.sort.imported')}</option>
        </select>
      </label>
      <div aria-label={message('library.viewLabel')} role="group">
        <button
          aria-pressed={view === 'large'}
          type="button"
          onClick={() => onViewChange('large')}
        >
          {message('library.view.large')}
        </button>
        <button
          aria-pressed={view === 'details'}
          type="button"
          onClick={() => onViewChange('details')}
        >
          {message('library.view.details')}
        </button>
      </div>
      <p aria-live="polite" className="library-command-bar__filter">
        {message('library.currentFilter', {
          filter: message(`library.filter.${filter}`),
        })}
      </p>
    </div>
  );
}
