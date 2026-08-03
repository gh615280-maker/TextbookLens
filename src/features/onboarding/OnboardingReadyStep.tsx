import type { BookSummary } from '../../lib/generated/book';
import type { OnboardingStateDto } from '../../lib/generated/onboarding';
interface Props {
  book: BookSummary;
  state: OnboardingStateDto;
  onStart(): void;
}
export function OnboardingReadyStep({ book, state, onStart }: Props) {
  const ready = book.importStatus === 'ready';
  return (
    <section aria-labelledby="onboarding-ready-title">
      <h2 id="onboarding-ready-title">Ready to read</h2>
      <dl>
        <dt>Book</dt>
        <dd>{book.title}</dd>
        <dt>Local import</dt>
        <dd>{ready ? 'Ready' : 'Still importing locally'}</dd>
        <dt>AI learning</dt>
        <dd>
          {state.learningProfileConnected ? 'Connected' : 'Not connected'}
        </dd>
        <dt>Vision</dt>
        <dd>{state.visionProfileConnected ? 'Connected' : 'Set up later'}</dd>
      </dl>
      <button type="button" disabled={!ready} onClick={onStart}>
        Start reading
      </button>
      {!ready ? (
        <p aria-live="polite">Import continues while you complete setup.</p>
      ) : null}
    </section>
  );
}
