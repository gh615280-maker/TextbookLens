import { useMessage } from '../../app/LanguageProvider';
import type { IndexRunAggregateDto } from '../../lib/generated/indexing';

export function IndexStatusSummary({
  aggregate,
}: {
  aggregate: IndexRunAggregateDto;
}) {
  const message = useMessage();
  const { pages } = aggregate;
  return (
    <section aria-labelledby="index-status-summary-title">
      <h2 id="index-status-summary-title">
        {message('indexQuality.summary.title')}
      </h2>
      <p role="status" aria-live="polite">
        {message('indexQuality.summary.overall', {
          aggregate: message(
            `indexQuality.status.${aggregate.aggregateStatus}`,
          ),
          control: message(`indexQuality.status.${aggregate.controlStatus}`),
        })}
      </p>
      <dl>
        <div>
          <dt>{message('indexQuality.summary.completed')}</dt>
          <dd>{pages.indexed + pages.notRequired}</dd>
        </div>
        <div>
          <dt>{message('indexQuality.summary.needsReview')}</dt>
          <dd>{pages.needsReview}</dd>
        </div>
        <div>
          <dt>{message('indexQuality.summary.failed')}</dt>
          <dd>{pages.failed}</dd>
        </div>
      </dl>
    </section>
  );
}
