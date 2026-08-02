import { formatMessage } from '../../lib/i18n';

export function OnboardingPage() {
  return (
    <section aria-labelledby="onboarding-title" className="phase-page">
      <h1 id="onboarding-title">
        {formatMessage('zh-CN', 'page.onboarding.title')}
      </h1>
      <p>{formatMessage('zh-CN', 'page.onboarding.description')}</p>
    </section>
  );
}
