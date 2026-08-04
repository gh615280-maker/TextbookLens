import { useEffect, useRef, useState } from 'react';

import { toUserError, type UserFacingError } from '../../lib/errors';
import type { TeachingApi, TeachingTestEvent } from './api';

type TestStage = 'idle' | 'running' | 'completed' | 'cancelled' | 'error';

const MAX_ANSWER_CODE_POINTS = 4_000;

export function TeachingTestPanel({
  api,
  instruction,
  enabled,
}: {
  api: TeachingApi;
  instruction: string;
  enabled: boolean;
}) {
  const [open, setOpen] = useState(false);
  const [question, setQuestion] = useState('');
  const [answer, setAnswer] = useState('');
  const [stage, setStage] = useState<TestStage>('idle');
  const [error, setError] = useState<UserFacingError | null>(null);
  const [usage, setUsage] = useState<{ input?: number; output?: number }>({});
  const [sessionId] = useState(() => crypto.randomUUID());
  const activeRequestId = useRef<string | null>(null);

  const cancelActive = () => {
    const requestId = activeRequestId.current;
    activeRequestId.current = null;
    if (!requestId) return;
    setStage('cancelled');
    void api.cancelTest?.(sessionId, requestId);
  };

  useEffect(() => {
    let mounted = true;
    let unlisten: (() => void) | undefined;
    if (api.listenTest) {
      void api
        .listenTest((event) => {
          if (!mounted || activeRequestId.current !== event.requestId) return;
          handleEvent(event, setAnswer, setUsage, setStage, setError);
          if (
            event.type === 'completed' ||
            event.type === 'cancelled' ||
            event.type === 'error'
          ) {
            activeRequestId.current = null;
          }
        })
        .then((stop) => {
          unlisten = stop;
        });
    }
    return () => {
      mounted = false;
      const requestId = activeRequestId.current;
      activeRequestId.current = null;
      if (requestId) void api.cancelTest?.(sessionId, requestId);
      unlisten?.();
    };
  }, [api, sessionId]);

  const start = async () => {
    if (!enabled || !api.startTest || !question.trim()) return;
    cancelActive();
    const requestId = crypto.randomUUID();
    activeRequestId.current = requestId;
    setAnswer('');
    setUsage({});
    setError(null);
    setStage('running');
    try {
      await api.startTest({
        sessionId,
        requestId,
        instruction,
        question,
      });
    } catch (reason) {
      if (activeRequestId.current !== requestId) return;
      activeRequestId.current = null;
      setStage('error');
      setError(toUserError(reason));
    }
  };

  return (
    <section aria-labelledby="teaching-test-title">
      <h2 id="teaching-test-title">Temporary test</h2>
      <p>This preview is temporary and is not saved to learning history.</p>
      <button
        aria-expanded={open}
        onClick={() => setOpen((value) => !value)}
        type="button"
      >
        {open ? 'Hide test' : 'Show test'}
      </button>
      {open ? (
        <fieldset disabled={!enabled}>
          <label htmlFor="teaching-test-question">Test question</label>
          <textarea
            id="teaching-test-question"
            maxLength={500}
            onChange={(event) => setQuestion(event.target.value)}
            rows={4}
            value={question}
          />
          <button
            disabled={stage === 'running' || !question.trim()}
            onClick={() => void start()}
            type="button"
          >
            Run temporary test
          </button>
          {stage === 'running' ? (
            <button onClick={cancelActive} type="button">
              Stop test
            </button>
          ) : null}
          {!enabled ? (
            <p>A default learning profile must be available.</p>
          ) : null}
        </fieldset>
      ) : null}
      <p aria-live="polite" data-testid="teaching-test-stage" role="status">
        {stage}
      </p>
      {error ? <p role="alert">{error.message}</p> : null}
      {answer ? (
        <output aria-label="Temporary test answer">{answer}</output>
      ) : null}
      {usage.input !== undefined || usage.output !== undefined ? (
        <p>
          Usage: {usage.input ?? 0} input / {usage.output ?? 0} output tokens
        </p>
      ) : null}
    </section>
  );
}

function handleEvent(
  event: TeachingTestEvent,
  setAnswer: (value: string | ((previous: string) => string)) => void,
  setUsage: (value: { input?: number; output?: number }) => void,
  setStage: (stage: TestStage) => void,
  setError: (error: UserFacingError | null) => void,
) {
  switch (event.type) {
    case 'text_delta':
      setAnswer((previous) =>
        Array.from(`${previous}${event.text ?? ''}`)
          .slice(0, MAX_ANSWER_CODE_POINTS)
          .join(''),
      );
      break;
    case 'usage':
      setUsage({ input: event.inputTokens, output: event.outputTokens });
      break;
    case 'completed':
      setStage('completed');
      break;
    case 'cancelled':
      setStage('cancelled');
      break;
    case 'error':
      setStage('error');
      setError({
        code: event.code ?? 'PROVIDER_UNAVAILABLE',
        message: 'The temporary test could not finish.',
        nextStep: 'Try again after checking the learning profile.',
        diagnosticId: null,
      });
      break;
  }
}
