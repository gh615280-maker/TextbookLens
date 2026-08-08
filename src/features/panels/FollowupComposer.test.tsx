import { fireEvent, render, screen } from '@testing-library/react';
import type { PropsWithChildren } from 'react';
import { describe, expect, it, vi } from 'vitest';

import { LanguageContext } from '../../app/LanguageProvider';
import { formatMessage } from '../../lib/i18n';
import { FollowupComposer } from './FollowupComposer';

describe('FollowupComposer', () => {
  it('preserves a concise provider-change notice and submits bounded question text', () => {
    const onSubmit = vi.fn();
    render(
      <EnglishLanguage>
        <FollowupComposer
          providerChangeNotice="Using Current / model; history used Previous / model."
          onSubmit={onSubmit}
        />
      </EnglishLanguage>,
    );
    fireEvent.change(screen.getByRole('textbox'), {
      target: { value: ' Why? ' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Send' }));
    expect(onSubmit).toHaveBeenCalledWith('Why?');
    expect(screen.getByRole('note')).toHaveTextContent('Previous');
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
