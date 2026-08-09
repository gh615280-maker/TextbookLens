import { useEffect, useRef, useState } from 'react';
import { useMessage } from '../../app/LanguageProvider';
import type {
  ProviderCapabilityRegistryDto,
  ProviderKind,
} from '../../lib/generated/provider';
import { ProviderAdvancedSettings } from './ProviderAdvancedSettings';
import type { SaveProviderProfileRequest } from './api';

const PROVIDERS: ProviderKind[] = [
  'openai',
  'gemini',
  'anthropic',
  'deepseek',
  'kimi',
];
const PROVIDER_NAMES: Record<ProviderKind, string> = {
  openai: 'OpenAI',
  gemini: 'Google Gemini',
  anthropic: 'Anthropic',
  deepseek: 'DeepSeek',
  kimi: 'Kimi',
};
interface Props {
  registry: ProviderCapabilityRegistryDto;
  busy: boolean;
  onConnect(
    request: SaveProviderProfileRequest,
    signal: AbortSignal,
  ): Promise<void>;
}

export function ProviderConnectForm({ registry, busy, onConnect }: Props) {
  const message = useMessage();
  const [kind, setKind] = useState<ProviderKind>('openai');
  const provider =
    registry.providers.find((item) => item.kind === kind) ??
    registry.providers[0];
  const [modelId, setModelId] = useState(provider.defaultModel);
  const [credential, setCredential] = useState('');
  const activeAttempt = useRef<AbortController | null>(null);
  useEffect(() => () => activeAttempt.current?.abort(), []);
  if (!provider) return null;
  async function submit(event: React.FormEvent) {
    event.preventDefault();
    if (busy || credential.length === 0 || activeAttempt.current) return;
    const attempt = new AbortController();
    activeAttempt.current = attempt;
    const secret = credential;
    try {
      await onConnect(
        {
          providerKind: kind,
          displayName: PROVIDER_NAMES[kind],
          modelId,
          credential: secret,
        },
        attempt.signal,
      );
      if (!attempt.signal.aborted) setCredential('');
    } catch {
      // The mounted field retains the key solely so the user can correct it.
    } finally {
      if (activeAttempt.current === attempt) activeAttempt.current = null;
    }
  }
  return (
    <form
      onSubmit={(event) => void submit(event)}
      aria-label={message('aiServices.connect.formLabel')}
    >
      <h2>{message('aiServices.connect.title')}</h2>
      <label>
        {message('aiServices.provider')}
        <select
          value={kind}
          onChange={(event) => {
            const nextKind = event.target.value as ProviderKind;
            activeAttempt.current?.abort();
            activeAttempt.current = null;
            setCredential('');
            setKind(nextKind);
            setModelId(
              registry.providers.find((item) => item.kind === nextKind)
                ?.defaultModel ?? '',
            );
          }}
        >
          {PROVIDERS.map((item) => (
            <option key={item} value={item}>
              {PROVIDER_NAMES[item]}
            </option>
          ))}
        </select>
      </label>
      <label>
        {message('aiServices.key')}
        <input
          aria-label={message('aiServices.key')}
          type="password"
          value={credential}
          onChange={(event) => setCredential(event.target.value)}
          autoComplete="new-password"
          spellCheck={false}
          required
        />
      </label>
      <ProviderAdvancedSettings
        provider={provider}
        modelId={modelId}
        onModelChange={setModelId}
      />
      <button type="submit" disabled={busy || credential.length === 0}>
        {message('aiServices.validateConnect')}
      </button>
    </form>
  );
}
