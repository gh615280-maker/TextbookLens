import type { UserFacingError } from '../../lib/errors';
import type { ImportEvent } from './parser-contract';

export type ImportViewState =
  | { status: 'idle' }
  | { status: 'running'; bookId: string | null; event: ImportEvent }
  | { status: 'failed'; error: UserFacingError };

export type ImportViewAction =
  | { type: 'start' }
  | { type: 'identify'; bookId: string }
  | { type: 'progress'; event: ImportEvent }
  | { type: 'failed'; error: UserFacingError }
  | { type: 'finish' };

export const initialImportViewState: ImportViewState = { status: 'idle' };

export function importViewReducer(
  state: ImportViewState,
  action: ImportViewAction,
): ImportViewState {
  switch (action.type) {
    case 'start':
      return {
        status: 'running',
        bookId: null,
        event: {
          stage: 'copying',
          completed: 0,
          total: 0,
          messageKey: 'import.copying',
        },
      };
    case 'identify':
      return state.status === 'running'
        ? { ...state, bookId: action.bookId }
        : state;
    case 'progress':
      return state.status === 'running'
        ? { ...state, event: action.event }
        : state;
    case 'failed':
      return { status: 'failed', error: action.error };
    case 'finish':
      return initialImportViewState;
  }
}
