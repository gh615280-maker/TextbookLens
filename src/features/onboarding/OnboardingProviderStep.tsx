import { useEffect, useState } from 'react';
import { toUserError, type UserFacingError } from '../../lib/errors';
import type { ProviderCapabilityRegistryDto } from '../../lib/generated/provider';
import { ProviderConnectForm } from '../providers/ProviderConnectForm';
import {
  TauriProviderApi,
  type ProviderApi,
  type SaveProviderProfileRequest,
} from '../providers/api';

interface Props {
  api?: ProviderApi;
  onConnected(): Promise<void>;
}
export function OnboardingProviderStep({
  api: suppliedApi,
  onConnected,
}: Props) {
  const [api] = useState<ProviderApi>(
    () => suppliedApi ?? new TauriProviderApi(),
  );
  const [registry, setRegistry] =
    useState<ProviderCapabilityRegistryDto | null>(null);
  const [error, setError] = useState<UserFacingError | null>(null);
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    void api
      .listCapabilities()
      .then(setRegistry)
      .catch((reason) => setError(toUserError(reason)));
  }, [api]);
  async function connect(request: SaveProviderProfileRequest) {
    setBusy(true);
    setError(null);
    try {
      await api.validateAndSave(request);
      await onConnected();
    } catch (reason) {
      setError(toUserError(reason));
      throw reason;
    } finally {
      setBusy(false);
    }
  }
  return (
    <section aria-labelledby="onboarding-provider-title">
      <h2 id="onboarding-provider-title">Connect an AI service</h2>
      <p>
        Choose a provider and validate its key. The model is selected
        automatically from verified provider information.
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
