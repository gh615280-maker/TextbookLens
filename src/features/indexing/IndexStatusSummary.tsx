import type { IndexRunAggregateDto } from '../../lib/generated/indexing';

export function IndexStatusSummary({
  aggregate,
}: {
  aggregate: IndexRunAggregateDto;
}) {
  const { pages } = aggregate;
  return (
    <section aria-labelledby="index-status-summary-title">
      <h2 id="index-status-summary-title">Index status</h2>
      <p role="status" aria-live="polite">
        Overall: {aggregate.aggregateStatus.replaceAll('_', ' ')}. Control:{' '}
        {aggregate.controlStatus}.
      </p>
      <dl>
        <div>
          <dt>Completed</dt>
          <dd>{pages.indexed + pages.notRequired}</dd>
        </div>
        <div>
          <dt>Needs review</dt>
          <dd>{pages.needsReview}</dd>
        </div>
        <div>
          <dt>Failed</dt>
          <dd>{pages.failed}</dd>
        </div>
      </dl>
    </section>
  );
}
