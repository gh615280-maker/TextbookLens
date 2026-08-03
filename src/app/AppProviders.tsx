import type { ReactNode } from 'react';

import { ErrorBoundary } from './ErrorBoundary';
import { LanguageProvider } from './LanguageProvider';

interface AppProvidersProps {
  children: ReactNode;
}

export function AppProviders({ children }: AppProvidersProps) {
  return (
    <LanguageProvider>
      <ErrorBoundary>{children}</ErrorBoundary>
    </LanguageProvider>
  );
}
