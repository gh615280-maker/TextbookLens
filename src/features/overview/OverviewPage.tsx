import { formatMessage } from '../../lib/i18n';

export function OverviewPage() {
  return (
    <section aria-labelledby="overview-title" className="phase-page">
      <h1 id="overview-title">
        {formatMessage('zh-CN', 'page.overview.title')}
      </h1>
      <p>{formatMessage('zh-CN', 'page.overview.description')}</p>
    </section>
  );
}
