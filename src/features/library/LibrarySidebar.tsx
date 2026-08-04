import { useMessage } from '../../app/LanguageProvider';

import type { LibraryFilter } from './library-state';

interface LibrarySidebarProps {
  filter: LibraryFilter;
  onFilterChange(filter: LibraryFilter): void;
}

const filters: readonly LibraryFilter[] = ['all', 'recent', 'processing'];

export function LibrarySidebar({
  filter,
  onFilterChange,
}: LibrarySidebarProps) {
  const message = useMessage();

  return (
    <nav
      aria-label={message('library.sidebarLabel')}
      className="library-sidebar"
      style={{
        display: 'flex',
        flexDirection: 'column',
        gap: 'var(--space-2)',
      }}
    >
      {filters.map((candidate) => (
        <button
          key={candidate}
          aria-pressed={filter === candidate}
          type="button"
          onClick={() => onFilterChange(candidate)}
        >
          {message(`library.filter.${candidate}`)}
        </button>
      ))}
    </nav>
  );
}
