import { useEffect, useRef, useState } from 'react';

import { useMessage } from '../../app/LanguageProvider';
import type { ProviderProfileSummary } from '../../lib/generated/provider';

interface Props {
  profile: ProviderProfileSummary;
  onReset(profileId: string): Promise<void>;
  busy: boolean;
}

export function ProviderConsentSettings({ profile, onReset, busy }: Props) {
  const message = useMessage();
  const [confirming, setConfirming] = useState(false);
  const trigger = useRef<HTMLButtonElement>(null);
  const confirmation = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    if (confirming) confirmation.current?.focus();
  }, [confirming]);
  async function reset() {
    await onReset(profile.id);
    setConfirming(false);
    trigger.current?.focus();
  }
  return (
    <section
      aria-label={message('aiServices.consent.region', {
        provider: profile.displayName,
      })}
    >
      <p>{message('aiServices.consent.description')}</p>
      <button
        ref={trigger}
        type="button"
        disabled={busy}
        onClick={() => setConfirming(true)}
      >
        {message('aiServices.consent.resetPrompts')}
      </button>
      {confirming ? (
        <div
          role="alertdialog"
          aria-modal="true"
          aria-label={message('aiServices.consent.dialog')}
        >
          <p>{message('aiServices.consent.confirm')}</p>
          <button ref={confirmation} type="button" onClick={() => void reset()}>
            {message('aiServices.consent.reset')}
          </button>
          <button
            type="button"
            onClick={() => {
              setConfirming(false);
              trigger.current?.focus();
            }}
          >
            {message('aiServices.cancel')}
          </button>
        </div>
      ) : null}
    </section>
  );
}
