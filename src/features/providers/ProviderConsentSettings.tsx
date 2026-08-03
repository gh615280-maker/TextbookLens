import { useEffect, useRef, useState } from 'react';
import type { ProviderProfileSummary } from '../../lib/generated/provider';
interface Props {
  profile: ProviderProfileSummary;
  onReset(profileId: string): Promise<void>;
  busy: boolean;
}
export function ProviderConsentSettings({ profile, onReset, busy }: Props) {
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
    <section aria-label={`${profile.displayName} consent settings`}>
      <p>
        These choices only control prompts for future actions you start. They
        never send images, start AI indexing, or perform network work.
      </p>
      <button
        ref={trigger}
        type="button"
        disabled={busy}
        onClick={() => setConfirming(true)}
      >
        Reset consent prompts
      </button>
      {confirming ? (
        <div
          role="alertdialog"
          aria-modal="true"
          aria-label="Reset consent prompts"
        >
          <p>Reset this profile’s consent prompts to Ask?</p>
          <button ref={confirmation} type="button" onClick={() => void reset()}>
            Reset
          </button>
          <button
            type="button"
            onClick={() => {
              setConfirming(false);
              trigger.current?.focus();
            }}
          >
            Cancel
          </button>
        </div>
      ) : null}
    </section>
  );
}
