import { describe, expect, it } from 'vitest';
import type { OnboardingStateDto } from '../../lib/generated/onboarding';
import {
  initialOnboardingViewState,
  onboardingReducer,
  visibleOnboardingStep,
} from './onboarding-state';

const book = {
  id: '4f9a2c86-0da8-4dd4-a255-39b4cff89c66',
  title: 'Synthetic textbook',
  originalFilename: 'synthetic.pdf',
  author: null,
  language: 'en',
  format: 'pdf' as const,
  importStatus: 'parsing' as const,
  importErrorCode: null,
  importErrorMessage: null,
  importErrorStage: null,
  readingProgress: 0,
  createdAt: '2026-08-03T00:00:00Z',
  updatedAt: '2026-08-03T00:00:00Z',
  lastOpenedAt: null,
};
const bootstrap: OnboardingStateDto = {
  step: 'provider',
  selectedBook: book,
  hasReadyBook: false,
  learningProfileConnected: false,
  visionProfileConnected: false,
  localTextQuality: 'pending',
  canSkipOnboarding: false,
};
describe('onboarding state', () => {
  it('keeps an identified import while provider setup is shown', () => {
    const loaded = onboardingReducer(initialOnboardingViewState, {
      type: 'loaded',
      bootstrap,
    });
    expect(visibleOnboardingStep(loaded)).toBe('provider');
    expect(loaded.selectedBook?.id).toBe(book.id);
  });
  it('uses durable book and credential facts rather than onboardingCompleted', () => {
    const ready = onboardingReducer(initialOnboardingViewState, {
      type: 'loaded',
      bootstrap: {
        ...bootstrap,
        selectedBook: { ...book, importStatus: 'ready' },
        hasReadyBook: true,
        learningProfileConnected: true,
        canSkipOnboarding: true,
      },
    });
    expect(visibleOnboardingStep(ready)).toBe('ready');
  });
});
