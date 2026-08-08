import { render, screen } from '@testing-library/react';
import type { PropsWithChildren } from 'react';
import { describe, expect, it } from 'vitest';

import { LanguageContext } from '../../app/LanguageProvider';
import { formatMessage } from '../../lib/i18n';
import { CitationList } from './CitationList';

describe('CitationList', () => {
  it('shows only supplied safe citation DTO fields', () => {
    render(
      <EnglishLanguage>
        <CitationList
          citations={[{ id: 'TL-C1', label: 'Page 2', quoteable: true }]}
          emptyLabel="None"
        />
      </EnglishLanguage>,
    );
    expect(screen.getByLabelText('Citations')).toHaveTextContent('Page 2');
  });
});

function EnglishLanguage({ children }: PropsWithChildren) {
  return (
    <LanguageContext.Provider
      value={{
        uiLanguage: 'en',
        isLoading: false,
        statusMessage: null,
        switchLanguage: async () => {},
        message: (key, values) => formatMessage('en', key, values),
      }}
    >
      {children}
    </LanguageContext.Provider>
  );
}
