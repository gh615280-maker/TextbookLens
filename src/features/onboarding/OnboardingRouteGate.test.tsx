import { cleanup, render, screen } from '@testing-library/react';
import { MemoryRouter, Route, Routes, useLocation } from 'react-router-dom';
import { afterEach, describe, expect, it } from 'vitest';

import { LanguageContext } from '../../app/LanguageProvider';
import { formatMessage } from '../../lib/i18n';
import type { OnboardingStateDto } from '../../lib/generated/onboarding';
import { OnboardingRouteGate } from './OnboardingRouteGate';
import type { OnboardingApi } from './onboarding-api';

const fresh: OnboardingStateDto = {
  step: 'book',
  selectedBook: null,
  hasReadyBook: false,
  learningProfileConnected: false,
  visionProfileConnected: false,
  localTextQuality: 'unavailable',
  canSkipOnboarding: false,
};
const eligible: OnboardingStateDto = {
  ...fresh,
  step: 'ready',
  hasReadyBook: true,
  learningProfileConnected: true,
  canSkipOnboarding: true,
};
function Location() {
  return <output data-testid="location">{useLocation().pathname}</output>;
}
function renderRoutes(state: OnboardingStateDto, initial = '/library') {
  const api: OnboardingApi = { getState: async () => state };
  return render(
    <LanguageContext.Provider
      value={{
        uiLanguage: 'en',
        isLoading: false,
        statusMessage: null,
        switchLanguage: async () => {},
        message: (key, values) => formatMessage('en', key, values),
      }}
    >
      <MemoryRouter initialEntries={[initial]}>
        <Routes>
          <Route path="/onboarding" element={<h1>Onboarding content</h1>} />
          <Route
            path="/library"
            element={
              <OnboardingRouteGate api={api}>
                <h1>Library content</h1>
              </OnboardingRouteGate>
            }
          />
        </Routes>
        <Location />
      </MemoryRouter>
    </LanguageContext.Provider>,
  );
}
afterEach(cleanup);
describe('OnboardingRouteGate', () => {
  it('redirects a fresh direct library entry before rendering library content', async () => {
    renderRoutes(fresh);
    expect(screen.queryByText('Library content')).not.toBeInTheDocument();
    expect(
      await screen.findByRole('heading', { name: 'Onboarding content' }),
    ).toBeVisible();
    expect(screen.getByTestId('location')).toHaveTextContent('/onboarding');
  });
  it('allows a ready book with an available default learning credential into library', async () => {
    renderRoutes(eligible);
    expect(
      await screen.findByRole('heading', { name: 'Library content' }),
    ).toBeVisible();
    expect(screen.getByTestId('location')).toHaveTextContent('/library');
  });
  it('does not gate an explicit onboarding route', async () => {
    renderRoutes(fresh, '/onboarding');
    expect(
      await screen.findByRole('heading', { name: 'Onboarding content' }),
    ).toBeVisible();
  });
});
