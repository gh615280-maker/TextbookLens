import type { UserFacingError } from '../../lib/errors';
import type { TeachingInstructionDto } from '../../lib/generated/teaching';

export type TeachingStage =
  'loading' | 'saved' | 'saving' | 'conflict' | 'error';

export interface TeachingState {
  stage: TeachingStage;
  persisted: TeachingInstructionDto | null;
  draft: string;
  dirty: boolean;
  learningProfileAvailable: boolean;
  error: UserFacingError | null;
}

export const initialTeachingState: TeachingState = {
  stage: 'loading',
  persisted: null,
  draft: '',
  dirty: false,
  learningProfileAvailable: false,
  error: null,
};

type TeachingAction =
  | { type: 'loading' }
  | {
      type: 'loaded';
      instruction: TeachingInstructionDto;
      learningProfileAvailable: boolean;
    }
  | { type: 'draftChanged'; draft: string }
  | { type: 'saving' }
  | { type: 'saved'; instruction: TeachingInstructionDto }
  | { type: 'conflict'; draft: string }
  | { type: 'preserveConflictDraft' }
  | { type: 'error'; error: UserFacingError };

export function teachingReducer(
  state: TeachingState,
  action: TeachingAction,
): TeachingState {
  switch (action.type) {
    case 'loading':
      return { ...state, stage: 'loading', error: null };
    case 'loaded':
      return {
        stage: 'saved',
        persisted: action.instruction,
        draft: action.instruction.instruction,
        dirty: false,
        learningProfileAvailable: action.learningProfileAvailable,
        error: null,
      };
    case 'draftChanged':
      return {
        ...state,
        stage: 'saved',
        draft: action.draft,
        dirty: action.draft !== state.persisted?.instruction,
        error: null,
      };
    case 'saving':
      return { ...state, stage: 'saving', error: null };
    case 'saved':
      return {
        ...state,
        stage: 'saved',
        persisted: action.instruction,
        draft: action.instruction.instruction,
        dirty: false,
        error: null,
      };
    case 'conflict':
      return { ...state, stage: 'conflict', draft: action.draft, dirty: true };
    case 'preserveConflictDraft':
      return { ...state, stage: 'saved', dirty: true };
    case 'error':
      return { ...state, stage: 'error', error: action.error };
  }
}
