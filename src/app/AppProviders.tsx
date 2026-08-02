import type { ReactNode } from 'react';

import { ErrorBoundary } from './ErrorBoundary';

interface AppProvidersProps {
  children: ReactNode;
}

export function AppProviders({ children }: AppProvidersProps) {
  return <ErrorBoundary>{children}</ErrorBoundary>;
}
