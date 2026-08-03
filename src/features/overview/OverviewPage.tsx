import { useMessage } from '../../app/LanguageProvider';

export function OverviewPage() {
  const message = useMessage();
  return (
    <section aria-labelledby="overview-title" className="phase-page">
      <h1 id="overview-title">{message('page.overview.title')}</h1>
      <p>{message('page.overview.description')}</p>
    </section>
  );
}
