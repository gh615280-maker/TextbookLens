interface Props {
  hasCompatibleVisionProfile: boolean;
  onContinue(): void;
}

/** This step deliberately has no provider, consent, upload, or indexing command. */
export function OnboardingVisualTextStep({
  hasCompatibleVisionProfile,
  onContinue,
}: Props) {
  const message = useMessage();
  return (
    <section aria-labelledby="onboarding-visual-title">
      <h2 id="onboarding-visual-title">{message('onboarding.visual.title')}</h2>
      <p>{message('onboarding.visual.description')}</p>
      {hasCompatibleVisionProfile ? (
        <p>{message('onboarding.visual.connected')}</p>
      ) : (
        <p>{message('onboarding.visual.later')}</p>
      )}
      <button type="button" onClick={onContinue}>
        {message('onboarding.visual.continue')}
      </button>
    </section>
  );
}
import { useMessage } from '../../app/LanguageProvider';
