import { useEffect, useRef, useState } from 'react';

import { useMessage } from '../../app/LanguageProvider';
import { useModalFocus } from '../../components/useModalFocus';
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

export function ProviderProfileRow(props: Props) {
  const message = useMessage();
  const { profile, registry } = props;
  const local = profile.kind === 'ollama' || profile.kind === 'lm_studio';
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
  const deleteDialogRef = useModalFocus<HTMLDivElement>(
    mode === 'delete',
    close,
  );
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
        <span>
          {local
            ? message('localModels.offline')
            : profile.credentialStatus === 'available'
              ? message('aiServices.connected')
              : message('aiServices.credentialMissing')}
        </span>{' '}
        · {profile.modelId}
      </p>
      <p>
        {message('aiServices.capability.text')}:{' '}
        {capabilityLabel(
          message,
          local ? 'supported' : (model?.textChat ?? 'unknown'),
        )}{' '}
        · {message('aiServices.capability.vision')}:{' '}
        {capabilityLabel(
          message,
          model?.imageInput ?? (local ? 'unsupported' : 'unknown'),
        )}{' '}
        · {message('aiServices.capability.structured')}:{' '}
        {capabilityLabel(
          message,
          local ? 'unsupported' : (model?.strictStructuredOutput ?? 'unknown'),
        )}
      </p>
      {props.learningDefault || props.visionDefault ? (
        <p>
          {props.learningDefault ? message('aiServices.learningDefault') : ''}
          {props.learningDefault && props.visionDefault ? ' · ' : ''}
          {props.visionDefault ? message('aiServices.visionDefault') : ''}
        </p>
      ) : null}
      <button
        type="button"
        disabled={props.busy}
        onClick={() => void props.onDefault('text_learning', profile.id)}
      >
        {message('aiServices.useForLearning')}
      </button>
      <button
        type="button"
        disabled={props.busy || model?.imageInput !== 'supported'}
        onClick={() => void props.onDefault('vision_learning', profile.id)}
      >
        {message('aiServices.useForVision')}
      </button>
      {!local ? (
        <button
          ref={trigger}
          type="button"
          disabled={props.busy}
          onClick={() => setMode('replace')}
        >
          {message('aiServices.replaceKey')}
        </button>
      ) : null}
      <button
        ref={local ? trigger : undefined}
        type="button"
        disabled={props.busy}
        onClick={() => setMode('delete')}
      >
        {message('aiServices.delete')}
      </button>
      {mode === 'replace' ? (
        <form
          onSubmit={(event) => void replace(event)}
          aria-label={message('aiServices.replaceForm', {
            provider: profile.displayName,
          })}
        >
          <label>
            {message('aiServices.newKey')}
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
            {message('aiServices.validateReplace')}
          </button>
          <button type="button" onClick={close}>
            {message('aiServices.cancel')}
          </button>
        </form>
      ) : null}
      {mode === 'delete' ? (
        <div
          ref={deleteDialogRef}
          role="alertdialog"
          aria-modal="true"
          aria-label={message('aiServices.deleteDialog', {
            provider: profile.displayName,
          })}
        >
          <p>{message('aiServices.deleteConfirm')}</p>
          <button
            ref={confirmation}
            type="button"
            onClick={() => void props.onDelete(profile.id).finally(close)}
          >
            {message('aiServices.delete')}
          </button>
          <button type="button" onClick={close}>
            {message('aiServices.cancel')}
          </button>
        </div>
      ) : null}
      {!local ? (
        <ProviderConsentSettings
          profile={profile}
          busy={props.busy}
          onReset={props.onResetConsents}
        />
      ) : null}
    </article>
  );
}

function capabilityLabel(
  message: ReturnType<typeof useMessage>,
  value: 'supported' | 'unsupported' | 'unknown',
) {
  if (value === 'supported') return message('aiServices.capability.supported');
  if (value === 'unsupported')
    return message('aiServices.capability.unsupported');
  return message('aiServices.capability.unknown');
}
