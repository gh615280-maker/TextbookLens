import { Navigate, type RouteObject } from 'react-router-dom';

import { AppLayout } from '../components/AppLayout';
import { AppShell, RouteErrorBoundary } from '../components/AppShell';
import { OnboardingShell } from '../components/OnboardingShell';
import { ReaderShell } from '../components/ReaderShell';
import { LibraryPage } from '../features/library/LibraryPage';
import { OnboardingPage } from '../features/onboarding/OnboardingPage';
import { OnboardingRouteGate } from '../features/onboarding/OnboardingRouteGate';
import { OverviewPage } from '../features/overview/OverviewPage';
import { AiServicesPage } from '../features/providers/AiServicesPage';
import { ReaderPage } from '../features/reader/ReaderPage';
import { SettingsPage } from '../features/settings/SettingsPage';
import { TeachingInstructionsPage } from '../features/teaching/TeachingInstructionsPage';
import { IndexQualityPage } from '../features/indexing/IndexQualityPage';
import { IndexStartPage } from '../features/indexing/IndexStartPage';

export const appRoutes: RouteObject[] = [
  {
    element: <AppLayout />,
    errorElement: <RouteErrorBoundary />,
    children: [
      {
        index: true,
        element: (
          <OnboardingRouteGate>
            <Navigate replace to="/library" />
          </OnboardingRouteGate>
        ),
      },
      {
        element: <OnboardingShell />,
        children: [{ path: '/onboarding', element: <OnboardingPage /> }],
      },
      {
        element: <AppShell />,
        children: [
          {
            path: '/library',
            element: (
              <OnboardingRouteGate>
                <LibraryPage />
              </OnboardingRouteGate>
            ),
          },
          {
            path: '/teaching-instructions',
            element: <TeachingInstructionsPage />,
          },
          { path: '/ai-services', element: <AiServicesPage /> },
          { path: '/books/:bookId/overview', element: <OverviewPage /> },
          { path: '/settings', element: <SettingsPage /> },
          {
            path: '/books/:bookId/index-quality/:runId',
            element: <IndexQualityPage />,
          },
          {
            path: '/books/:bookId/index-start',
            element: <IndexStartPage />,
          },
        ],
      },
      {
        element: <ReaderShell />,
        children: [{ path: '/books/:bookId/read', element: <ReaderPage /> }],
      },
    ],
  },
];
