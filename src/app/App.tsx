import { useMemo } from 'react';
import {
  createBrowserRouter,
  createMemoryRouter,
  RouterProvider,
} from 'react-router-dom';

import { AppProviders } from './AppProviders';
import { appRoutes } from './router';

interface AppProps {
  initialEntries?: string[];
}

export function App({ initialEntries }: AppProps) {
  const router = useMemo(
    () =>
      initialEntries
        ? createMemoryRouter(appRoutes, { initialEntries })
        : createBrowserRouter(appRoutes),
    [initialEntries],
  );

  return (
    <AppProviders>
      <RouterProvider router={router} />
    </AppProviders>
  );
}
