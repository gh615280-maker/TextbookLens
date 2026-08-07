import type { BookSummary } from '../../lib/generated/book';

export type LibraryFilter = 'all' | 'recent' | 'processing';
export type LibrarySort = 'title' | 'lastOpened' | 'imported';
export type LibraryView = 'large' | 'details';

export interface LibraryState {
  filter: LibraryFilter;
  query: string;
  sort: LibrarySort;
  view: LibraryView;
  selectedBookId: string | null;
  focusedBookId: string | null;
}

export type LibraryAction =
  | { type: 'setFilter'; filter: LibraryFilter }
  | { type: 'setQuery'; query: string }
  | { type: 'setSort'; sort: LibrarySort }
  | { type: 'setView'; view: LibraryView }
  | { type: 'select'; bookId: string }
  | { type: 'focus'; bookId: string | null };

export const initialLibraryState: LibraryState = {
  filter: 'all',
  query: '',
  sort: 'title',
  view: 'large',
  selectedBookId: null,
  focusedBookId: null,
};

export function libraryStateReducer(
  state: LibraryState,
  action: LibraryAction,
): LibraryState {
  switch (action.type) {
    case 'setFilter':
      return { ...state, filter: action.filter };
    case 'setQuery':
      return { ...state, query: action.query };
    case 'setSort':
      return { ...state, sort: action.sort };
    case 'setView':
      return { ...state, view: action.view };
    case 'select':
      return {
        ...state,
        selectedBookId: action.bookId,
        focusedBookId: action.bookId,
      };
    case 'focus':
      return { ...state, focusedBookId: action.bookId };
  }
}

export function isBookOpenable(book: BookSummary): boolean {
  return book.importStatus === 'ready';
}

export function isProcessingBook(
  book: BookSummary,
  activeIndexBookIds: ReadonlySet<string> = new Set(),
): boolean {
  return (
    activeIndexBookIds.has(book.id) ||
    book.importStatus === 'queued' ||
    book.importStatus === 'copying' ||
    book.importStatus === 'parsing' ||
    book.importStatus === 'indexing'
  );
}

export function getVisibleBooks(
  books: readonly BookSummary[],
  state: Pick<LibraryState, 'filter' | 'query' | 'sort'>,
  locale: string,
  activeIndexBookIds: ReadonlySet<string> = new Set(),
): BookSummary[] {
  const query = foldForSearch(state.query, locale);
  return [...books]
    .filter((book) => matchesFilter(book, state.filter, activeIndexBookIds))
    .filter((book) => matchesQuery(book, query, locale))
    .sort((left, right) => compareBooks(left, right, state.sort, locale));
}

export function foldForSearch(value: string, locale: string): string {
  return value.normalize('NFKC').toLocaleLowerCase(locale).trim();
}

function matchesFilter(
  book: BookSummary,
  filter: LibraryFilter,
  activeIndexBookIds: ReadonlySet<string>,
): boolean {
  switch (filter) {
    case 'all':
      return true;
    case 'recent':
      return book.lastOpenedAt !== null;
    case 'processing':
      return isProcessingBook(book, activeIndexBookIds);
  }
}

function matchesQuery(
  book: BookSummary,
  query: string,
  locale: string,
): boolean {
  if (!query) return true;
  return [book.title, book.originalFilename, book.author]
    .filter((value): value is string => value !== null)
    .some((value) => foldForSearch(value, locale).includes(query));
}

function compareBooks(
  left: BookSummary,
  right: BookSummary,
  sort: LibrarySort,
  locale: string,
): number {
  let compared = 0;
  switch (sort) {
    case 'title':
      compared = left.title.localeCompare(right.title, locale, {
        sensitivity: 'base',
        numeric: true,
      });
      break;
    case 'lastOpened':
      compared = compareNewestFirst(left.lastOpenedAt, right.lastOpenedAt);
      break;
    case 'imported':
      compared = compareNewestFirst(left.createdAt, right.createdAt);
      break;
  }
  return compared || left.id.localeCompare(right.id, 'en');
}

function compareNewestFirst(left: string | null, right: string | null): number {
  if (left === right) return 0;
  if (left === null) return 1;
  if (right === null) return -1;
  return right.localeCompare(left, 'en');
}
