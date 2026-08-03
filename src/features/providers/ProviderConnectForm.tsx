import { useEffect, useState } from 'react';
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
  onConnect(request: SaveProviderProfileRequest): Promise<void>;
}

export function ProviderConnectForm({ registry, busy, onConnect }: Props) {
  const [kind, setKind] = useState<ProviderKind>('openai');
  const provider =
    registry.providers.find((item) => item.kind === kind) ??
    registry.providers[0];
  const [modelId, setModelId] = useState(provider.defaultModel);
  const [credential, setCredential] = useState('');
  useEffect(() => () => setCredential(''), []);
  if (!provider) return null;
  async function submit(event: React.FormEvent) {
    event.preventDefault();
    const secret = credential;
    try {
      await onConnect({
        providerKind: kind,
        displayName: PROVIDER_NAMES[kind],
        modelId,
        credential: secret,
      });
      setCredential('');
    } catch {
      // The mounted field retains the key solely so the user can correct it.
    }
  }
  return (
    <form
      onSubmit={(event) => void submit(event)}
      aria-label="Connect AI provider"
    >
      <h2>Connect an AI service</h2>
      <label>
        Provider
        <select
          value={kind}
          onChange={(event) => {
            const nextKind = event.target.value as ProviderKind;
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
        Key
        <input
          aria-label="Key"
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
        Validate &amp; Connect
      </button>
    </form>
  );
}
