import { formatMessage } from '../../lib/i18n';

export function ReaderPage() {
  return (
    <section
      aria-labelledby="reader-title"
      className="phase-page reader-column"
    >
      <h1 id="reader-title">{formatMessage('zh-CN', 'page.reader.title')}</h1>
      <p>{formatMessage('zh-CN', 'page.reader.description')}</p>
    </section>
  );
}
