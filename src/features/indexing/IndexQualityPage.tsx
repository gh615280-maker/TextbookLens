import { useCallback, useEffect, useMemo, useState } from 'react';
import { useParams } from 'react-router-dom';

import { useMessage } from '../../app/LanguageProvider';
import { toUserError, type UserFacingError } from '../../lib/errors';
import type {
  IndexPageReviewDto,
  IndexRunAggregateDto,
} from '../../lib/generated/indexing';
import { useIndexingCoordinator } from './IndexingProvider';
import { IndexPageList } from './IndexPageList';
import { IndexReviewEditor } from './IndexReviewEditor';
import { IndexStatusSummary } from './IndexStatusSummary';
import { TauriIndexingApi, type IndexingApi } from './api';

export function IndexQualityPage({
  api: suppliedApi,
}: { api?: IndexingApi } = {}) {
  const { bookId, runId } = useParams();
  const message = useMessage();
  const [api] = useState<IndexingApi>(
    () => suppliedApi ?? new TauriIndexingApi(),
  );
  const coordinator = useIndexingCoordinator();
  const [aggregate, setAggregate] = useState<IndexRunAggregateDto | null>(null);
  const [pages, setPages] = useState<IndexPageReviewDto[]>([]);
  const [selectedPageId, setSelectedPageId] = useState<string | null>(null);
  const [error, setError] = useState<UserFacingError | null>(null);
  const [busy, setBusy] = useState(false);
  const selected = useMemo(
    () => pages.find((page) => page.id === selectedPageId) ?? null,
    [pages, selectedPageId],
  );

  const load = useCallback(async () => {
    if (!runId) return;
    try {
      const [nextAggregate, reviews] = await Promise.all([
        api.getRunAggregate(runId),
        api.listPageReviews(runId),
      ]);
      if (bookId && nextAggregate.bookId !== bookId)
        throw new Error(message('indexQuality.bookMismatch'));
      setAggregate(nextAggregate);
      setPages(
        reviews.filter(
          (page) => page.status === 'needs_review' || page.status === 'failed',
        ),
      );
      setSelectedPageId((current) =>
        reviews.some((page) => page.id === current)
          ? current
          : (reviews[0]?.id ?? null),
      );
      setError(null);
    } catch (cause) {
      setError(toUserError(cause));
    }
  }, [api, bookId, message, runId]);

  useEffect(() => {
    void Promise.resolve().then(load);
  }, [load]);
  useEffect(() => {
    const unsubscribe = coordinator.subscribe((event) => {
      if (event.runId === runId) void load();
    });
    const timer = window.setInterval(() => void load(), 3_000);
    return () => {
      unsubscribe();
      window.clearInterval(timer);
    };
  }, [coordinator, load, runId]);

  async function control(action: 'pause' | 'resume' | 'cancel') {
    if (!runId || busy) return;
    setBusy(true);
    try {
      await {
        pause: api.pauseRun,
        resume: api.resumeRun,
        cancel: api.cancelRun,
      }[action].call(api, runId);
      await load();
    } catch (cause) {
      setError(toUserError(cause));
    } finally {
      setBusy(false);
    }
  }

  if (!runId || !bookId)
    return <p role="alert">{message('indexQuality.routeMissing')}</p>;
  return (
    <section aria-labelledby="index-quality-title" className="phase-page">
      <h1 id="index-quality-title">{message('indexQuality.title')}</h1>
      <p>{message('indexQuality.description')}</p>
      {error ? (
        <div role="alert">
          <p>{error.message}</p>
          <p>{error.nextStep}</p>
          <button type="button" onClick={() => void load()}>
            {message('indexQuality.retryLoading')}
          </button>
        </div>
      ) : null}
      {aggregate ? (
        <>
          <IndexStatusSummary aggregate={aggregate} />
          <div aria-label={message('indexQuality.controls')} role="group">
            {aggregate.controlStatus === 'running' ? (
              <button
                disabled={busy}
                type="button"
                onClick={() => void control('pause')}
              >
                {message('indexQuality.pause')}
              </button>
            ) : null}
            {aggregate.controlStatus === 'paused' ? (
              <button
                disabled={busy}
                type="button"
                onClick={() => void control('resume')}
              >
                {message('indexQuality.resume')}
              </button>
            ) : null}
            {aggregate.controlStatus === 'running' ||
            aggregate.controlStatus === 'paused' ? (
              <button
                disabled={busy}
                type="button"
                onClick={() => void control('cancel')}
              >
                {message('indexQuality.cancel')}
              </button>
            ) : null}
            <p aria-live="polite">
              {busy ? message('indexQuality.updating') : ''}
            </p>
          </div>
        </>
      ) : (
        <p aria-live="polite">{message('indexQuality.loading')}</p>
      )}
      <IndexPageList
        pages={pages}
        selectedPageId={selectedPageId}
        onSelect={setSelectedPageId}
      />
      {selected ? (
        <IndexReviewEditor
          page={selected}
          api={api}
          onChanged={() => void load()}
        />
      ) : null}
    </section>
  );
}
