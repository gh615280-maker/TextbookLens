import { useEffect, useRef, useState } from 'react';

import { useMessage } from '../../app/LanguageProvider';
import { useModalFocus } from '../../components/useModalFocus';
import type { ConfirmIndexOperationRequest, IndexingApi } from './api';

export interface IndexStartConfirmationProps {
  request: ConfirmIndexOperationRequest;
  profileName: string;
  modelName: string;
  api: Pick<IndexingApi, 'confirmOperation' | 'createRun' | 'authorizeRun'>;
  onSetProfileNoPrompt?(profileId: string): Promise<void>;
  onReject(): void;
  onStarted(runId: string): void;
}

/** A user must activate this dialog once for every run, even when prompts are skipped later. */
export function IndexStartConfirmation({
  request,
  profileName,
  modelName,
  api,
  onSetProfileNoPrompt,
  onReject,
  onStarted,
}: IndexStartConfirmationProps) {
  const message = useMessage();
  const [noPrompt, setNoPrompt] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const mountedRef = useRef(true);
  const inFlightRef = useRef(false);
  const dialogRef = useModalFocus<HTMLDivElement>(true, () => {
    if (!inFlightRef.current) onReject();
  });

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  async function confirm() {
    if (inFlightRef.current) return;
    inFlightRef.current = true;
    setBusy(true);
    setError(null);
    try {
      // This always obtains a new, single-use token from the durable coordinator.
      const token = await api.confirmOperation(request);
      if (!mountedRef.current) return;
      if (noPrompt) await onSetProfileNoPrompt?.(request.providerProfileId);
      if (!mountedRef.current) return;
      const runId = request.runId;
      if (runId) {
        await api.authorizeRun(runId, token, request);
      } else {
        const createdRunId = await api.createRun(token, request);
        if (mountedRef.current) onStarted(createdRunId);
        return;
      }
      if (mountedRef.current) onStarted(runId);
    } catch {
      if (mountedRef.current) setError(message('indexStart.startFailed'));
    } finally {
      inFlightRef.current = false;
      if (mountedRef.current) setBusy(false);
    }
  }

  return (
    <div
      ref={dialogRef}
      aria-describedby="index-start-description"
      aria-labelledby="index-start-confirm-title"
      aria-modal="true"
      role="dialog"
    >
      <h2 id="index-start-confirm-title">
        {message('indexStart.confirmTitle')}
      </h2>
      <p id="index-start-description">
        {message('indexStart.details', {
          profile: profileName,
          model: modelName,
          pages: request.pages.length,
        })}
      </p>
      <p>{message('indexStart.sent', { pages: request.pages.length })}</p>
      <label>
        <input
          checked={noPrompt}
          disabled={busy}
          type="checkbox"
          onChange={(event) => setNoPrompt(event.currentTarget.checked)}
        />
        {message('indexStart.noPrompt')}
      </label>
      {error ? <p role="alert">{error}</p> : null}
      <div className="button-row">
        <button disabled={busy} type="button" onClick={onReject}>
          {message('indexStart.reject')}
        </button>
        <button disabled={busy} type="button" onClick={() => void confirm()}>
          {busy
            ? message('indexStart.starting')
            : message('indexStart.confirm')}
        </button>
      </div>
    </div>
  );
}
