import { useMessage } from '../../app/LanguageProvider';

export function SettingsPage() {
  const message = useMessage();
  return (
    <section aria-labelledby="settings-title" className="phase-page">
      <h1 id="settings-title">{message('page.settings.title')}</h1>
      <p>{message('page.settings.description')}</p>
    </section>
  );
}
