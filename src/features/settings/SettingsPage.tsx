import { formatMessage } from '../../lib/i18n';

export function SettingsPage() {
  return (
    <section aria-labelledby="settings-title" className="phase-page">
      <h1 id="settings-title">
        {formatMessage('zh-CN', 'page.settings.title')}
      </h1>
      <p>{formatMessage('zh-CN', 'page.settings.description')}</p>
    </section>
  );
}
