import { useState } from 'react';
import { useMessage } from '../../app/LanguageProvider';
import type { BookSummary } from '../../lib/generated/book';
import type { UserFacingError } from '../../lib/errors';
import { ImportButton } from '../import/ImportButton';
import type { ImportCoordinatorPort } from '../import/ImportCoordinator';

interface Props {
  coordinator: ImportCoordinatorPort;
  onBookIdentified(book: BookSummary): void;
}
export function OnboardingBookStep({ coordinator, onBookIdentified }: Props) {
  const message = useMessage();
  const [error, setError] = useState<UserFacingError | null>(null);
  async function select(sourcePath: string) {
    setError(null);
    try {
      const book = await coordinator.importDocument(
        sourcePath,
        () => {},
        onBookIdentified,
      );
      onBookIdentified(book);
    } catch (reason) {
      setError(reason as UserFacingError);
    }
  }
  return (
    <section aria-labelledby="onboarding-book-title">
      <h2 id="onboarding-book-title">{message('onboarding.book')}</h2>
      <p>{message('onboarding.book.description')}</p>
      <ImportButton onSelect={select} />
      {error ? (
        <div role="alert">
          <p>{error.message}</p>
          <p>{error.nextStep}</p>
        </div>
      ) : null}
    </section>
  );
}
