import { useEffect, useRef, useState } from 'react';
import type {
  ProviderCapabilityRegistryDto,
  ProviderProfileSummary,
} from '../../lib/generated/provider';
import { ProviderConsentSettings } from './ProviderConsentSettings';

interface Props {
  profile: ProviderProfileSummary;
  registry: ProviderCapabilityRegistryDto;
  learningDefault: boolean;
  visionDefault: boolean;
  busy: boolean;
  onDefault(
    operation: 'text_learning' | 'vision_learning',
    id: string,
  ): Promise<void>;
  onDelete(id: string): Promise<void>;
  onReplace(id: string, key: string): Promise<void>;
  onResetConsents(id: string): Promise<void>;
}
const label = (value: 'supported' | 'unsupported' | 'unknown') =>
  value === 'supported'
    ? 'Supported'
    : value === 'unsupported'
      ? 'Unsupported'
      : 'Unknown';
export function ProviderProfileRow(props: Props) {
  const { profile, registry } = props;
  const [mode, setMode] = useState<'none' | 'delete' | 'replace'>('none');
  const [key, setKey] = useState('');
  const trigger = useRef<HTMLButtonElement>(null);
  const confirmation = useRef<HTMLButtonElement>(null);
  const replacementInput = useRef<HTMLInputElement>(null);
  const model = registry.providers
    .find((provider) => provider.kind === profile.kind)
    ?.models.find((item) => item.id === profile.modelId);
  const close = () => {
    setKey('');
    setMode('none');
    trigger.current?.focus();
  };
  useEffect(() => {
    if (mode === 'delete') confirmation.current?.focus();
    if (mode === 'replace') replacementInput.current?.focus();
  }, [mode]);
  async function replace(event: React.FormEvent) {
    event.preventDefault();
    try {
      await props.onReplace(profile.id, key);
    } finally {
      close();
    }
  }
  return (
    <article aria-label={profile.displayName}>
      <h3>{profile.displayName}</h3>
      <p>
        {profile.credentialStatus === 'available'
          ? 'Connected'
          : 'Credential missing'}{' '}
        · {profile.modelId}
      </p>
      <p>
        Text: {label(model?.textChat ?? 'unknown')} · Vision:{' '}
        {label(model?.imageInput ?? 'unknown')} · Structured:{' '}
        {label(model?.strictStructuredOutput ?? 'unknown')}
      </p>
      <p>
        {props.learningDefault ? 'Learning default' : ''}
        {props.learningDefault && props.visionDefault ? ' · ' : ''}
        {props.visionDefault ? 'Vision default' : ''}
      </p>
      <button
        type="button"
        disabled={props.busy}
        onClick={() => void props.onDefault('text_learning', profile.id)}
      >
        Use for learning
      </button>
      <button
        type="button"
        disabled={props.busy || model?.imageInput !== 'supported'}
        onClick={() => void props.onDefault('vision_learning', profile.id)}
      >
        Use for vision
      </button>
      <button
        ref={trigger}
        type="button"
        disabled={props.busy}
        onClick={() => setMode('replace')}
      >
        Replace key
      </button>
      <button
        type="button"
        disabled={props.busy}
        onClick={() => setMode('delete')}
      >
        Delete
      </button>
      {mode === 'replace' ? (
        <form
          onSubmit={(event) => void replace(event)}
          aria-label={`Replace key for ${profile.displayName}`}
        >
          <label>
            New key
            <input
              ref={replacementInput}
              type="password"
              value={key}
              onChange={(event) => setKey(event.target.value)}
              autoComplete="new-password"
              required
            />
          </label>
          <button type="submit" disabled={props.busy || key.length === 0}>
            Validate &amp; Replace
          </button>
          <button type="button" onClick={close}>
            Cancel
          </button>
        </form>
      ) : null}
      {mode === 'delete' ? (
        <div
          role="alertdialog"
          aria-modal="true"
          aria-label={`Delete ${profile.displayName}`}
        >
          <p>Delete this provider profile and its saved key?</p>
          <button
            ref={confirmation}
            type="button"
            onClick={() => void props.onDelete(profile.id).finally(close)}
          >
            Delete
          </button>
          <button type="button" onClick={close}>
            Cancel
          </button>
        </div>
      ) : null}
      <ProviderConsentSettings
        profile={profile}
        busy={props.busy}
        onReset={props.onResetConsents}
      />
    </article>
  );
}
