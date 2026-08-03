import { useMessage } from '../../app/LanguageProvider';

export function OnboardingPage() {
  const message = useMessage();
  return (
    <section aria-labelledby="onboarding-title" className="phase-page">
      <h1 id="onboarding-title">{message('page.onboarding.title')}</h1>
      <p>{message('page.onboarding.description')}</p>
    </section>
  );
}
