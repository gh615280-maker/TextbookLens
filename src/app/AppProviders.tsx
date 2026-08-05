import type { ReactNode } from 'react';

import { ErrorBoundary } from './ErrorBoundary';
import { LanguageProvider } from './LanguageProvider';
import { IndexingProvider } from '../features/indexing/IndexingProvider';
import { LearningRequestProvider } from '../features/learning/LearningRequestProvider';
import { FloatingPanelHost } from '../features/panels/FloatingPanelHost';

interface AppProvidersProps {
  children: ReactNode;
}

export function AppProviders({ children }: AppProvidersProps) {
  return (
    <LanguageProvider>
      <LearningRequestProvider>
        <IndexingProvider>
          <ErrorBoundary>
            {children}
            <FloatingPanelHost />
          </ErrorBoundary>
        </IndexingProvider>
      </LearningRequestProvider>
    </LanguageProvider>
  );
}
