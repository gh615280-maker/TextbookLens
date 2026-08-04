/* eslint-disable react-refresh/only-export-components */

import {
  createContext,
  useContext,
  useEffect,
  useState,
  type ReactNode,
} from 'react';

import { IndexingCoordinator } from './IndexingCoordinator';

const IndexingCoordinatorContext = createContext<IndexingCoordinator | null>(
  null,
);

interface IndexingProviderProps {
  children: ReactNode;
  coordinator?: IndexingCoordinator;
}

export function IndexingProvider({
  children,
  coordinator: suppliedCoordinator,
}: IndexingProviderProps) {
  const [coordinator] = useState(
    () => suppliedCoordinator ?? new IndexingCoordinator(),
  );

  useEffect(() => {
    coordinator.start();
    return () => {
      // Provider teardown means app shutdown. Do not mutate a durable run: Rust
      // startup recovery will resume any claimed attempt.
      void coordinator.stop();
    };
  }, [coordinator]);

  return (
    <IndexingCoordinatorContext.Provider value={coordinator}>
      {children}
    </IndexingCoordinatorContext.Provider>
  );
}

export function useIndexingCoordinator(): IndexingCoordinator {
  const coordinator = useContext(IndexingCoordinatorContext);
  if (!coordinator) {
    throw new Error(
      'useIndexingCoordinator must be used inside IndexingProvider',
    );
  }
  return coordinator;
}
