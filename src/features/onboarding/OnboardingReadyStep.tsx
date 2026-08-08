import type { BookSummary } from '../../lib/generated/book';
import type { OnboardingStateDto } from '../../lib/generated/onboarding';
import { useMessage } from '../../app/LanguageProvider';
interface Props {
  book: BookSummary;
  state: OnboardingStateDto;
  onStart(): void;
}
export function OnboardingReadyStep({ book, state, onStart }: Props) {
  const message = useMessage();
  const ready = book.importStatus === 'ready';
  return (
    <section aria-labelledby="onboarding-ready-title">
      <h2 id="onboarding-ready-title">{message('onboarding.ready')}</h2>
      <dl>
        <dt>{message('onboarding.ready.book')}</dt>
        <dd>{book.title}</dd>
        <dt>{message('onboarding.ready.localImport')}</dt>
        <dd>
          {message(
            ready
              ? 'onboarding.ready.localReady'
              : 'onboarding.ready.localPending',
          )}
        </dd>
        <dt>{message('onboarding.ready.aiLearning')}</dt>
        <dd>
          {message(
            state.learningProfileConnected
              ? 'onboarding.ready.connected'
              : 'onboarding.ready.notConnected',
          )}
        </dd>
        <dt>{message('onboarding.ready.vision')}</dt>
        <dd>
          {message(
            state.visionProfileConnected
              ? 'onboarding.ready.connected'
              : 'onboarding.ready.setUpLater',
          )}
        </dd>
      </dl>
      <button type="button" disabled={!ready} onClick={onStart}>
        {message('onboarding.ready.start')}
      </button>
      {!ready ? (
        <p aria-live="polite">{message('onboarding.ready.importing')}</p>
      ) : null}
    </section>
  );
}
