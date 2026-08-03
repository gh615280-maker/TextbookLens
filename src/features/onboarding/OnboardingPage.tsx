import { useCallback, useEffect, useReducer, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useNavigate } from 'react-router-dom';
import { toUserError } from '../../lib/errors';
import type { OnboardingStateDto } from '../../lib/generated/onboarding';
import { TauriImportIpc } from '../../lib/ipc';
import {
  ImportCoordinator,
  type ImportCoordinatorPort,
} from '../import/ImportCoordinator';
import { createDocumentParserRegistry } from '../import/parser-registry';
import { OnboardingBookStep } from './OnboardingBookStep';
import { OnboardingProviderStep } from './OnboardingProviderStep';
import { OnboardingReadyStep } from './OnboardingReadyStep';
import {
  initialOnboardingViewState,
  onboardingReducer,
  visibleOnboardingStep,
} from './onboarding-state';

export interface OnboardingApi {
  getState(): Promise<OnboardingStateDto>;
}
class TauriOnboardingApi implements OnboardingApi {
  getState() {
    return invoke<OnboardingStateDto>('get_onboarding_state');
  }
}

export function OnboardingPage({
  api: suppliedApi,
  importCoordinator,
}: { api?: OnboardingApi; importCoordinator?: ImportCoordinatorPort } = {}) {
  const navigate = useNavigate();
  const [api] = useState<OnboardingApi>(
    () => suppliedApi ?? new TauriOnboardingApi(),
  );
  const [coordinator] = useState<ImportCoordinatorPort>(
    () =>
      importCoordinator ??
      new ImportCoordinator(
        new TauriImportIpc(),
        createDocumentParserRegistry(),
      ),
  );
  const [state, dispatch] = useReducer(
    onboardingReducer,
    initialOnboardingViewState,
  );
  const bootstrap = useCallback(async () => {
    try {
      dispatch({ type: 'loaded', bootstrap: await api.getState() });
    } catch (reason) {
      dispatch({ type: 'error', error: toUserError(reason) });
    }
  }, [api]);
  useEffect(() => {
    void bootstrap();
  }, [bootstrap]);
  if (state.stage === 'loading')
    return (
      <section className="phase-page">
        <p aria-live="polite">Loading setup…</p>
      </section>
    );
  if (state.error)
    return (
      <section className="phase-page">
        <div role="alert">
          <p>{state.error.message}</p>
          <button type="button" onClick={() => void bootstrap()}>
            Retry
          </button>
        </div>
      </section>
    );
  const step = visibleOnboardingStep(state);
  const book = state.selectedBook;
  return (
    <section aria-labelledby="onboarding-title" className="phase-page">
      <h1 id="onboarding-title">Get started</h1>
      {step === 'book' ? (
        <OnboardingBookStep
          coordinator={coordinator}
          onBookIdentified={(identified) =>
            dispatch({ type: 'book-selected', book: identified })
          }
        />
      ) : null}
      {step === 'provider' ? (
        <OnboardingProviderStep onConnected={bootstrap} />
      ) : null}
      {step === 'ready' && book && state.bootstrap ? (
        <OnboardingReadyStep
          book={book}
          state={state.bootstrap}
          onStart={() => navigate(`/books/${book.id}/read`)}
        />
      ) : null}
    </section>
  );
}
