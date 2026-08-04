import type { ReactNode } from 'react';

import { ErrorBoundary } from './ErrorBoundary';
import { LanguageProvider } from './LanguageProvider';
import { IndexingProvider } from '../features/indexing/IndexingProvider';

interface AppProvidersProps {
  children: ReactNode;
}

export function AppProviders({ children }: AppProvidersProps) {
  return (
    <LanguageProvider>
      <IndexingProvider>
        <ErrorBoundary>{children}</ErrorBoundary>
      </IndexingProvider>
    </LanguageProvider>
  );
}
