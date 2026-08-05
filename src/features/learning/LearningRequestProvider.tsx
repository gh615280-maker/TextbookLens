/* eslint-disable react-refresh/only-export-components */

import {
  createContext,
  useContext,
  useEffect,
  useMemo,
  useState,
  useSyncExternalStore,
  type ReactNode,
} from 'react';

import { TauriLearningApi, type LearningRequestApi } from './api';
import {
  LearningRequestStore,
  type LearningRequestStoreSnapshot,
} from './learning-request-store';
import type {
  LearningSurfacePort,
  PreparedLearningHandoff,
} from './selection-state';

interface LearningRequestContextValue {
  readonly store: LearningRequestStore;
  readonly surface: LearningSurfacePort;
}

const LearningRequestContext =
  createContext<LearningRequestContextValue | null>(null);
const EMPTY_SNAPSHOT: LearningRequestStoreSnapshot = Object.freeze({
  requests: Object.freeze([]),
});

interface LearningRequestProviderProps {
  children: ReactNode;
  api?: LearningRequestApi;
  store?: LearningRequestStore;
}

export function LearningRequestProvider({
  children,
  api: suppliedApi,
  store: suppliedStore,
}: LearningRequestProviderProps) {
  const api = useMemo(
    () => suppliedApi ?? new TauriLearningApi(),
    [suppliedApi],
  );
  const store = useMemo(
    () => suppliedStore ?? new LearningRequestStore(),
    [suppliedStore],
  );
  const [controller] = useState(() => new LearningRequestController());

  useEffect(() => {
    controller.activate();
    return () => controller.deactivate();
  }, [controller]);

  const value = useMemo<LearningRequestContextValue>(() => {
    const surface: LearningSurfacePort = Object.freeze({
      handoff: (prepared: Readonly<PreparedLearningHandoff>) =>
        controller.handoff(api, store, prepared),
    });
    return Object.freeze({ store, surface });
  }, [api, controller, store]);

  return (
    <LearningRequestContext.Provider value={value}>
      {children}
    </LearningRequestContext.Provider>
  );
}

class LearningRequestController {
  private active = true;
  private readonly subscriptions = new Set<() => void>();

  activate() {
    this.active = true;
  }

  deactivate() {
    this.active = false;
    for (const unsubscribe of this.subscriptions) unsubscribe();
    this.subscriptions.clear();
    // Deliberately do not cancel requests: only an explicit Stop may do that.
  }

  async handoff(
    api: LearningRequestApi,
    store: LearningRequestStore,
    prepared: Readonly<PreparedLearningHandoff>,
  ) {
    const started = await api.start(prepared.preparationId);
    if (!this.active) return;
    store.applySnapshot(started);
    void api
      .subscribe(started.requestId, started.lastSeq, (event) => {
        if (this.active) store.applyEvent(event);
      })
      .then(({ snapshot, unsubscribe }) => {
        if (!this.active) {
          unsubscribe();
          return;
        }
        this.subscriptions.add(unsubscribe);
        store.applySnapshot(snapshot);
      })
      .catch(() => {
        // The backend owns terminal state. A subscription failure never cancels it.
      });
  }
}

export function useLearningSurfacePort(): LearningSurfacePort | null {
  return useContext(LearningRequestContext)?.surface ?? null;
}

export function useLearningRequestSnapshot(): LearningRequestStoreSnapshot {
  const store = useContext(LearningRequestContext)?.store;
  return useSyncExternalStore(
    (listener) => store?.subscribe(listener) ?? (() => {}),
    () => store?.snapshot() ?? EMPTY_SNAPSHOT,
    () => EMPTY_SNAPSHOT,
  );
}
