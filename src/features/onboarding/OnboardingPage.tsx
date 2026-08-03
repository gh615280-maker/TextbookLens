import { useCallback, useEffect, useReducer, useRef, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { useMessage } from '../../app/LanguageProvider';
import { toUserError } from '../../lib/errors';
import { TauriImportIpc } from '../../lib/ipc';
import {
  ImportCoordinator,
  type ImportCoordinatorPort,
} from '../import/ImportCoordinator';
import { createDocumentParserRegistry } from '../import/parser-registry';
import { OnboardingBookStep } from './OnboardingBookStep';
import { OnboardingProviderStep } from './OnboardingProviderStep';
import { OnboardingReadyStep } from './OnboardingReadyStep';
import { OnboardingVisualTextStep } from './OnboardingVisualTextStep';
import { TauriOnboardingApi, type OnboardingApi } from './onboarding-api';
import {
  initialOnboardingViewState,
  onboardingReducer,
  visibleOnboardingStep,
} from './onboarding-state';
import type { ProviderApi } from '../providers/api';

export function OnboardingPage({
  api: suppliedApi,
  importCoordinator,
  providerApi,
}: {
  api?: OnboardingApi;
  importCoordinator?: ImportCoordinatorPort;
  providerApi?: ProviderApi;
} = {}) {
  const navigate = useNavigate();
  const message = useMessage();
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
  const mounted = useRef(false);
  const bootstrapSequence = useRef(0);
  const bootstrap = useCallback(async () => {
    const sequence = ++bootstrapSequence.current;
    try {
      const next = await api.getState();
      if (mounted.current && sequence === bootstrapSequence.current) {
        dispatch({ type: 'loaded', bootstrap: next });
      }
    } catch (reason) {
      if (mounted.current && sequence === bootstrapSequence.current) {
        dispatch({ type: 'error', error: toUserError(reason) });
      }
    }
  }, [api]);
  useEffect(() => {
    mounted.current = true;
    void bootstrap();
    return () => {
      mounted.current = false;
      bootstrapSequence.current += 1;
    };
  }, [bootstrap]);
  if (state.stage === 'loading')
    return (
      <section aria-labelledby="onboarding-title" className="phase-page">
        <h1 id="onboarding-title">{message('page.onboarding.title')}</h1>
        <p aria-live="polite">Loading setup…</p>
      </section>
    );
  if (state.error)
    return (
      <section aria-labelledby="onboarding-title" className="phase-page">
        <h1 id="onboarding-title">{message('page.onboarding.title')}</h1>
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
      <h1 id="onboarding-title">{message('page.onboarding.title')}</h1>
      {step === 'book' ? (
        <OnboardingBookStep
          coordinator={coordinator}
          onBookIdentified={(identified) =>
            dispatch({ type: 'book-selected', book: identified })
          }
        />
      ) : null}
      {step === 'provider' ? (
        <OnboardingProviderStep
          api={providerApi}
          book={book!}
          onConnected={bootstrap}
        />
      ) : null}
      {step === 'visual' && state.bootstrap ? (
        <OnboardingVisualTextStep
          hasCompatibleVisionProfile={state.bootstrap.visionProfileConnected}
          onContinue={() => dispatch({ type: 'visual-declined' })}
        />
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
