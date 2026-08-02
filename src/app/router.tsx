import { Navigate, type RouteObject } from 'react-router-dom';

import { AppLayout } from '../components/AppLayout';
import { LibraryPage } from '../features/library/LibraryPage';
import { OnboardingPage } from '../features/onboarding/OnboardingPage';
import { OverviewPage } from '../features/overview/OverviewPage';
import { ReaderPage } from '../features/reader/ReaderPage';
import { SettingsPage } from '../features/settings/SettingsPage';

export const appRoutes: RouteObject[] = [
  {
    element: <AppLayout />,
    children: [
      { index: true, element: <Navigate replace to="/library" /> },
      { path: '/onboarding', element: <OnboardingPage /> },
      { path: '/library', element: <LibraryPage /> },
      { path: '/books/:bookId/read', element: <ReaderPage /> },
      { path: '/books/:bookId/overview', element: <OverviewPage /> },
      { path: '/settings', element: <SettingsPage /> },
    ],
  },
];
