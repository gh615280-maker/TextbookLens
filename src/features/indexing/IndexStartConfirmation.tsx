import { useState } from 'react';

import type { ConfirmIndexOperationRequest, IndexingApi } from './api';

export interface IndexStartConfirmationProps {
  request: ConfirmIndexOperationRequest;
  profileName: string;
  modelName: string;
  api: Pick<IndexingApi, 'confirmOperation' | 'createRun'>;
  onSetProfileNoPrompt?(profileId: string): Promise<void>;
  onStarted(runId: string): void;
}

/** A user must activate this dialog once for every run, even when prompts are skipped later. */
export function IndexStartConfirmation({
  request,
  profileName,
  modelName,
  api,
  onSetProfileNoPrompt,
  onStarted,
}: IndexStartConfirmationProps) {
  const [noPrompt, setNoPrompt] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function confirm() {
    if (busy) return;
    setBusy(true);
    setError(null);
    try {
      // This always obtains a new, single-use token from the durable coordinator.
      const token = await api.confirmOperation(request);
      if (noPrompt) await onSetProfileNoPrompt?.(request.providerProfileId);
      const runId = await api.createRun(token, request);
      onStarted(runId);
    } catch {
      setError(
        'The index run could not be started. Review the profile and try again.',
      );
    } finally {
      setBusy(false);
    }
  }

  return (
    <div
      aria-describedby="index-start-description"
      aria-modal="true"
      role="dialog"
    >
      <h2>Start AI-assisted indexing?</h2>
      <p id="index-start-description">
        Profile: {profileName}. Model: {modelName}. Pages:{' '}
        {request.pages.length}.
      </p>
      <p>
        The local page images for these {request.pages.length} page
        {request.pages.length === 1 ? '' : 's'} will be sent to this provider
        for structured page analysis. This may incur provider charges.
      </p>
      <label>
        <input
          checked={noPrompt}
          disabled={busy}
          type="checkbox"
          onChange={(event) => setNoPrompt(event.currentTarget.checked)}
        />
        Do not show this index-start prompt again for this profile
      </label>
      {error ? <p role="alert">{error}</p> : null}
      <button disabled={busy} type="button" onClick={() => void confirm()}>
        {busy ? 'Starting index run…' : 'Confirm and start'}
      </button>
    </div>
  );
}
