import type {
  ProviderCapabilityRegistryDto,
  ProviderProfileSummary,
} from '../../lib/generated/provider';
import type { AppSettingsDto } from '../../lib/generated/settings';
import type { UserFacingError } from '../../lib/errors';

export interface ProviderViewState {
  stage: 'loading' | 'ready' | 'saving' | 'error';
  profiles: ProviderProfileSummary[];
  registry: ProviderCapabilityRegistryDto | null;
  settings: AppSettingsDto | null;
  error: UserFacingError | null;
}

export const initialProviderViewState: ProviderViewState = {
  stage: 'loading',
  profiles: [],
  registry: null,
  settings: null,
  error: null,
};

export type ProviderViewAction =
  | {
      type: 'loaded';
      profiles: ProviderProfileSummary[];
      registry: ProviderCapabilityRegistryDto;
      settings: AppSettingsDto;
    }
  | { type: 'saving' }
  | { type: 'error'; error: UserFacingError }
  | { type: 'done' };

// Intentionally holds safe server metadata and presentation stage only: never a credential.
export function providerViewReducer(
  state: ProviderViewState,
  action: ProviderViewAction,
): ProviderViewState {
  switch (action.type) {
    case 'loaded':
      return {
        stage: 'ready',
        profiles: action.profiles,
        registry: action.registry,
        settings: action.settings,
        error: null,
      };
    case 'saving':
      return { ...state, stage: 'saving', error: null };
    case 'error':
      return { ...state, stage: 'error', error: action.error };
    case 'done':
      return { ...state, stage: 'ready' };
  }
}
