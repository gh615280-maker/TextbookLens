import type { UserFacingError } from '../../lib/errors';
import type { OnboardingStateDto } from '../../lib/generated/onboarding';
import type { BookSummary } from '../../lib/generated/book';

export interface OnboardingViewState {
  stage: 'loading' | 'ready' | 'error';
  bootstrap: OnboardingStateDto | null;
  selectedBook: BookSummary | null;
  error: UserFacingError | null;
}
export const initialOnboardingViewState: OnboardingViewState = {
  stage: 'loading',
  bootstrap: null,
  selectedBook: null,
  error: null,
};
export type OnboardingAction =
  | { type: 'loaded'; bootstrap: OnboardingStateDto }
  | { type: 'book-selected'; book: BookSummary }
  | { type: 'error'; error: UserFacingError };
export function onboardingReducer(
  state: OnboardingViewState,
  action: OnboardingAction,
): OnboardingViewState {
  switch (action.type) {
    case 'loaded':
      return {
        stage: 'ready',
        bootstrap: action.bootstrap,
        selectedBook: action.bootstrap.selectedBook ?? state.selectedBook,
        error: null,
      };
    case 'book-selected':
      return {
        ...state,
        stage: 'ready',
        selectedBook: action.book,
        error: null,
      };
    case 'error':
      return { ...state, stage: 'error', error: action.error };
  }
}

export function visibleOnboardingStep(
  state: OnboardingViewState,
): 'book' | 'provider' | 'ready' {
  if (!state.selectedBook) return 'book';
  if (!state.bootstrap?.learningProfileConnected) return 'provider';
  return 'ready';
}
