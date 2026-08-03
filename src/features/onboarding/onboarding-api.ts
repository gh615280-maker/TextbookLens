import { invoke } from '@tauri-apps/api/core';

import type { OnboardingStateDto } from '../../lib/generated/onboarding';

export interface OnboardingApi {
  getState(): Promise<OnboardingStateDto>;
}

export class TauriOnboardingApi implements OnboardingApi {
  getState(): Promise<OnboardingStateDto> {
    return invoke<OnboardingStateDto>('get_onboarding_state');
  }
}
