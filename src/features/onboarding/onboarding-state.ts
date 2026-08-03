import type { UserFacingError } from '../../lib/errors';
import type { OnboardingStateDto } from '../../lib/generated/onboarding';
import type { BookSummary } from '../../lib/generated/book';

export interface OnboardingViewState {
  stage: 'loading' | 'ready' | 'error';
  bootstrap: OnboardingStateDto | null;
  selectedBook: BookSummary | null;
  visualDecision: 'undecided' | 'declined';
  error: UserFacingError | null;
}
export const initialOnboardingViewState: OnboardingViewState = {
  stage: 'loading',
  bootstrap: null,
  selectedBook: null,
  visualDecision: 'undecided',
  error: null,
};
export type OnboardingAction =
  | { type: 'loaded'; bootstrap: OnboardingStateDto }
  | { type: 'book-selected'; book: BookSummary }
  | { type: 'visual-declined' }
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
        visualDecision:
          action.bootstrap.localTextQuality === 'visual_setup_recommended'
            ? 'undecided'
            : 'declined',
        error: null,
      };
    case 'book-selected':
      return {
        ...state,
        stage: 'ready',
        selectedBook: action.book,
        visualDecision: 'undecided',
        error: null,
      };
    case 'error':
      return { ...state, stage: 'error', error: action.error };
    case 'visual-declined':
      return { ...state, visualDecision: 'declined' };
  }
}

export function visibleOnboardingStep(
  state: OnboardingViewState,
): 'book' | 'provider' | 'visual' | 'ready' {
  if (!state.selectedBook) return 'book';
  if (!state.bootstrap?.learningProfileConnected) return 'provider';
  if (
    state.bootstrap.localTextQuality === 'visual_setup_recommended' &&
    state.visualDecision === 'undecided'
  )
    return 'visual';
  return 'ready';
}
