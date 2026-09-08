import { useCallback, useEffect, useReducer, useState } from 'react';

import { useMessage } from '../../app/LanguageProvider';
import { toUserError } from '../../lib/errors';
import { ProviderConnectForm } from './ProviderConnectForm';
import { ProviderProfileRow } from './ProviderProfileRow';
import { LocalModelConnect } from './LocalModelConnect';
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
  const message = useMessage();
  const [api] = useState<ProviderApi>(
    () => providedApi ?? new TauriProviderApi(),
  );
  const [state, dispatch] = useReducer(
    providerViewReducer,
    initialProviderViewState,
  );
  const load = useCallback(
    async (signal?: AbortSignal) => {
      try {
        const [profiles, registry, settings] = await Promise.all([
          api.listProfiles(),
          api.listCapabilities(),
          api.getSettings(),
        ]);
        if (signal?.aborted) return;
        dispatch({ type: 'loaded', profiles, registry, settings });
      } catch (error) {
        if (signal?.aborted) return;
        dispatch({ type: 'error', error: toUserError(error) });
      }
    },
    [api],
  );
  useEffect(() => {
    void load();
  }, [load]);
  const mutate = async (
    action: () => Promise<unknown>,
    signal?: AbortSignal,
  ) => {
    dispatch({ type: 'saving' });
    try {
      await action();
      if (signal?.aborted) return;
      await load(signal);
    } catch (error) {
      if (signal?.aborted) return;
      dispatch({ type: 'error', error: toUserError(error) });
      throw error;
    } finally {
      if (signal?.aborted) dispatch({ type: 'done' });
    }
  };
  const registry = state.registry;
  const settings = state.settings;
  return (
    <section aria-labelledby="ai-services-title" className="phase-page">
      <h1 id="ai-services-title">{message('aiServices.title')}</h1>
      <p>{message('aiServices.description')}</p>
      <LocalModelConnect
        api={api}
        busy={state.stage === 'saving' || state.stage === 'loading'}
        onConnected={load}
        onBusyChange={(busy) => dispatch({ type: busy ? 'saving' : 'done' })}
      />
      {state.error ? (
        <div role="alert">
          <p>{message('aiServices.error')}</p>
          <p>{message('aiServices.errorNextStep')}</p>
        </div>
      ) : null}
      {registry ? (
        <ProviderConnectForm
          registry={registry}
          busy={state.stage === 'saving'}
          onConnect={(request: SaveProviderProfileRequest, signal) =>
            mutate(() => api.validateAndSave(request), signal)
          }
        />
      ) : (
        <p aria-live="polite">{message('aiServices.loading')}</p>
      )}
      <h2>{message('aiServices.connectedProfiles')}</h2>
      {registry && settings && state.profiles.length === 0 ? (
        <p>{message('aiServices.empty')}</p>
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
