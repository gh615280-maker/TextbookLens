import { useRef, useState } from 'react';
import { useMessage } from '../../app/LanguageProvider';
import type {
  LocalModelConnectResult,
  LocalServiceStatus,
} from '../../lib/generated/provider';
import type { MessageKey } from '../../lib/i18n';
import type { ProviderApi } from './api';

const statusMessages: Record<LocalServiceStatus, MessageKey> = {
  connected: 'localModels.status.connected',
  not_installed: 'localModels.status.notInstalled',
  unavailable: 'localModels.status.unavailable',
  authentication_required: 'localModels.status.authentication',
  no_models: 'localModels.status.noModels',
  no_usable_models: 'localModels.status.noUsableModels',
};

export function LocalModelConnect({
  api,
  busy,
  onConnected,
  onBusyChange,
}: {
  api: Pick<ProviderApi, 'connectLocal'>;
  busy: boolean;
  onConnected(): Promise<void>;
  onBusyChange(busy: boolean): void;
}) {
  const message = useMessage();
  const inFlight = useRef(false);
  const [running, setRunning] = useState(false);
  const [result, setResult] = useState<LocalModelConnectResult | null>(null);
  const [failed, setFailed] = useState(false);
  async function connect() {
    if (inFlight.current || busy) return;
    inFlight.current = true;
    setRunning(true);
    onBusyChange(true);
    setResult(null);
    setFailed(false);
    try {
      const result = await api.connectLocal();
      setResult(result);
      if (result.defaultProfileId) await onConnected();
    } catch {
      setFailed(true);
    } finally {
      inFlight.current = false;
      setRunning(false);
      onBusyChange(false);
    }
  }
  return (
    <section
      aria-labelledby="local-models-title"
      className="local-model-connect"
    >
      <h2 id="local-models-title">{message('localModels.title')}</h2>
      <p>{message('localModels.description')}</p>
      <button
        type="button"
        disabled={busy || running}
        onClick={() => void connect()}
      >
        {message(running ? 'localModels.connecting' : 'localModels.connect')}
      </button>
      <div aria-live="polite" role="status">
        {running ? <p>{message('localModels.wait')}</p> : null}
        {result?.defaultProfileId ? (
          <p>
            {message('localModels.success', { count: result.profiles.length })}
          </p>
        ) : null}
        {result?.services.map((service) => (
          <p key={service.kind}>
            {service.kind === 'ollama' ? 'Ollama' : 'LM Studio'} ·{' '}
            {message(statusMessages[service.status])}
          </p>
        ))}
      </div>
      {failed ? <p role="alert">{message('localModels.error')}</p> : null}
    </section>
  );
}
