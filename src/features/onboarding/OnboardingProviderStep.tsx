import { useEffect, useRef, useState } from 'react';
import { toUserError, type UserFacingError } from '../../lib/errors';
import type { BookSummary } from '../../lib/generated/book';
import type { ProviderCapabilityRegistryDto } from '../../lib/generated/provider';
import { ProviderConnectForm } from '../providers/ProviderConnectForm';
import {
  TauriProviderApi,
  type ProviderApi,
  type SaveProviderProfileRequest,
} from '../providers/api';

interface Props {
  api?: ProviderApi;
  book: BookSummary;
  onConnected(): Promise<void>;
}
export function OnboardingProviderStep({
  api: suppliedApi,
  book,
  onConnected,
}: Props) {
  const [api] = useState<ProviderApi>(
    () => suppliedApi ?? new TauriProviderApi(),
  );
  const [registry, setRegistry] =
    useState<ProviderCapabilityRegistryDto | null>(null);
  const [error, setError] = useState<UserFacingError | null>(null);
  const [busy, setBusy] = useState(false);
  const mounted = useRef(false);
  const inFlight = useRef<Promise<void> | null>(null);
  useEffect(() => {
    mounted.current = true;
    void api
      .listCapabilities()
      .then((capabilities) => {
        if (mounted.current) setRegistry(capabilities);
      })
      .catch((reason) => {
        if (mounted.current) setError(toUserError(reason));
      });
    return () => {
      mounted.current = false;
    };
  }, [api]);
  function connect(request: SaveProviderProfileRequest): Promise<void> {
    if (inFlight.current) return inFlight.current;
    const operation = (async () => {
      if (mounted.current) {
        setBusy(true);
        setError(null);
      }
      try {
        await api.validateAndSave(request);
        if (!mounted.current) throw new DOMException('Stale UI', 'AbortError');
        await onConnected();
      } catch (reason) {
        if (mounted.current) setError(toUserError(reason));
        throw reason;
      } finally {
        inFlight.current = null;
        if (mounted.current) setBusy(false);
      }
    })();
    inFlight.current = operation;
    return operation;
  }
  return (
    <section aria-labelledby="onboarding-provider-title">
      <h2 id="onboarding-provider-title">Connect an AI service</h2>
      <p>
        Choose a provider and validate its key. The model is selected
        automatically from verified provider information.
      </p>
      <p>
        Book: {book.title}. Its local import continues while the key is
        validated.
      </p>
      {error ? (
        <div role="alert">
          <p>{error.message}</p>
          <p>{error.nextStep}</p>
        </div>
      ) : null}
      {registry ? (
        <ProviderConnectForm
          registry={registry}
          busy={busy}
          onConnect={connect}
        />
      ) : (
        <p aria-live="polite">Loading providers…</p>
      )}
    </section>
  );
}
