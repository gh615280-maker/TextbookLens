import { useCallback, useEffect, useReducer, useState } from 'react';
import { toUserError } from '../../lib/errors';
import { ProviderConnectForm } from './ProviderConnectForm';
import { ProviderProfileRow } from './ProviderProfileRow';
import {
  TauriProviderApi,
  type ProviderApi,
  type SaveProviderProfileRequest,
} from './api';
import {
  initialProviderViewState,
  providerViewReducer,
} from './provider-view-state';

export function AiServicesPage({
  api: providedApi,
}: { api?: ProviderApi } = {}) {
  const [api] = useState<ProviderApi>(
    () => providedApi ?? new TauriProviderApi(),
  );
  const [state, dispatch] = useReducer(
    providerViewReducer,
    initialProviderViewState,
  );
  const load = useCallback(async () => {
    try {
      const [profiles, registry, settings] = await Promise.all([
        api.listProfiles(),
        api.listCapabilities(),
        api.getSettings(),
      ]);
      dispatch({ type: 'loaded', profiles, registry, settings });
    } catch (error) {
      dispatch({ type: 'error', error: toUserError(error) });
    }
  }, [api]);
  useEffect(() => {
    void load();
  }, [load]);
  const mutate = async (action: () => Promise<unknown>) => {
    dispatch({ type: 'saving' });
    try {
      await action();
      await load();
    } catch (error) {
      dispatch({ type: 'error', error: toUserError(error) });
      throw error;
    }
  };
  const registry = state.registry;
  const settings = state.settings;
  return (
    <section aria-labelledby="ai-services-title" className="phase-page">
      <h1 id="ai-services-title">AI services</h1>
      <p>
        Connect a provider to enable AI-assisted learning. Keys are validated
        before they are saved.
      </p>
      {state.error ? (
        <div role="alert">
          <p>{state.error.message}</p>
          <p>{state.error.nextStep}</p>
        </div>
      ) : null}
      {registry ? (
        <ProviderConnectForm
          registry={registry}
          busy={state.stage === 'saving'}
          onConnect={(request: SaveProviderProfileRequest) =>
            mutate(() => api.validateAndSave(request))
          }
        />
      ) : (
        <p aria-live="polite">Loading AI services…</p>
      )}
      <h2>Connected profiles</h2>
      {registry && settings && state.profiles.length === 0 ? (
        <p>No provider is connected yet.</p>
      ) : null}
      {registry && settings
        ? state.profiles.map((profile) => (
            <ProviderProfileRow
              key={profile.id}
              profile={profile}
              registry={registry}
              learningDefault={settings.defaultLearningProfileId === profile.id}
              visionDefault={settings.defaultVisionProfileId === profile.id}
              busy={state.stage === 'saving'}
              onDefault={(operation, id) =>
                mutate(() => api.setDefault(operation, id))
              }
              onDelete={(id) => mutate(() => api.deleteProfile(id))}
              onReplace={(id, key) =>
                mutate(() => api.replaceCredential(id, key))
              }
              onResetConsents={(id) => mutate(() => api.resetConsents(id))}
            />
          ))
        : null}
    </section>
  );
}
