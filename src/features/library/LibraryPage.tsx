import { formatMessage } from '../../lib/i18n';

export function LibraryPage() {
  return (
    <section aria-labelledby="library-title" className="phase-page">
      <h1 id="library-title">{formatMessage('zh-CN', 'page.library.title')}</h1>
      <p>{formatMessage('zh-CN', 'page.library.description')}</p>
    </section>
  );
}
