import { useCallback, useEffect, useState, type ReactNode } from 'react';
import { Navigate } from 'react-router-dom';

import { toUserError, type UserFacingError } from '../../lib/errors';
import { TauriOnboardingApi, type OnboardingApi } from './onboarding-api';

interface Props {
  children: ReactNode;
  api?: OnboardingApi;
}

/** Resolves every library entry from durable backend facts before rendering content. */
export function OnboardingRouteGate({ children, api: suppliedApi }: Props) {
  const [api] = useState<OnboardingApi>(
    () => suppliedApi ?? new TauriOnboardingApi(),
  );
  const [eligible, setEligible] = useState<boolean | null>(null);
  const [error, setError] = useState<UserFacingError | null>(null);
  const [attempt, setAttempt] = useState(0);
  const retry = useCallback(() => {
    setEligible(null);
    setError(null);
    setAttempt((current) => current + 1);
  }, []);
  useEffect(() => {
    let active = true;
    void api.getState().then(
      (state) => {
        if (active) setEligible(state.canSkipOnboarding);
      },
      (reason: unknown) => {
        if (active) setError(toUserError(reason));
      },
    );
    return () => {
      active = false;
    };
  }, [api, attempt]);

  if (error) {
    return (
      <section className="phase-page" aria-live="polite">
        <div role="alert">
          <p>{error.message}</p>
          <button type="button" onClick={retry}>
            Retry
          </button>
        </div>
      </section>
    );
  }
  if (eligible === null) {
    return (
      <section className="phase-page">
        <p aria-live="polite">Loading your library…</p>
      </section>
    );
  }
  return eligible ? <>{children}</> : <Navigate replace to="/onboarding" />;
}
