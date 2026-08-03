interface Props {
  hasCompatibleVisionProfile: boolean;
  onContinue(): void;
}

/** This step deliberately has no provider, consent, upload, or indexing command. */
export function OnboardingVisualTextStep({
  hasCompatibleVisionProfile,
  onContinue,
}: Props) {
  return (
    <section aria-labelledby="onboarding-visual-title">
      <h2 id="onboarding-visual-title">Local text is unavailable</h2>
      <p>
        This PDF has no extractable local text. Per-page text quality and visual
        processing are not started during setup.
      </p>
      {hasCompatibleVisionProfile ? (
        <p>A compatible visual profile is connected and can be used later.</p>
      ) : (
        <p>You can set up a compatible visual profile later.</p>
      )}
      <button type="button" onClick={onContinue}>
        Continue without visual setup
      </button>
    </section>
  );
}
