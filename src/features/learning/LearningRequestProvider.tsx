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
import { TauriBookLearningPreparationApi } from './book-api';
import {
  LearningRequestStore,
  type LearningRequestPresentation,
  type LearningRequestStoreSnapshot,
} from './learning-request-store';
import type {
  LearningSurfacePort,
  PreparedLearningHandoff,
} from './selection-state';

interface LearningRequestContextValue {
  readonly store: LearningRequestStore;
  readonly surface: LearningSurfacePort;
  startFollowup(
    conversationId: string,
    question: string,
    presentation: Readonly<LearningRequestPresentation>,
  ): Promise<void>;
  cancel(requestId: string): Promise<void>;
  startBook(
    preparationId: string,
    presentation: Readonly<LearningRequestPresentation>,
    targetConversationId?: string,
  ): Promise<void>;
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
  const [bookApi] = useState(() => new TauriBookLearningPreparationApi());

  useEffect(() => {
    controller.activate();
    return () => controller.deactivate();
  }, [controller]);

  const value = useMemo<LearningRequestContextValue>(() => {
    const surface: LearningSurfacePort = Object.freeze({
      handoff: (prepared: Readonly<PreparedLearningHandoff>) =>
        controller.handoff(api, store, prepared),
    });
    return Object.freeze({
      store,
      surface,
      startFollowup: (
        conversationId: string,
        question: string,
        presentation: Readonly<LearningRequestPresentation>,
      ) =>
        controller.startFollowup(
          api,
          store,
          conversationId,
          question,
          presentation,
        ),
      cancel: (requestId: string) => api.cancel(requestId),
      startBook: (
        preparationId: string,
        presentation: Readonly<LearningRequestPresentation>,
        targetConversationId?: string,
      ) =>
        controller.startBook(
          api,
          bookApi,
          store,
          preparationId,
          presentation,
          targetConversationId,
        ),
    });
  }, [api, bookApi, controller, store]);

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
    store.setPresentation(started.requestId, {
      action: prepared.action,
      selectionLabel: prepared.selectionLabel,
      provider: prepared.summary.providerDisplayName,
      model: prepared.summary.modelDisplayName,
    });
    this.subscribe(api, store, started);
  }

  async startFollowup(
    api: LearningRequestApi,
    store: LearningRequestStore,
    conversationId: string,
    question: string,
    presentation: Readonly<LearningRequestPresentation>,
  ) {
    const started = await api.startFollowup(conversationId, question);
    if (!this.active) return;
    store.applySnapshot(started);
    store.setTargetConversation(started.requestId, conversationId);
    store.setPresentation(started.requestId, presentation);
    this.subscribe(api, store, started);
  }

  async startBook(
    api: LearningRequestApi,
    bookApi: Pick<TauriBookLearningPreparationApi, 'start'>,
    store: LearningRequestStore,
    preparationId: string,
    presentation: Readonly<LearningRequestPresentation>,
    targetConversationId?: string,
  ) {
    const started = await bookApi.start(preparationId);
    if (!this.active) return;
    store.applySnapshot(started);
    if (targetConversationId) {
      store.setTargetConversation(started.requestId, targetConversationId);
    }
    store.setPresentation(started.requestId, presentation);
    this.subscribe(api, store, started);
  }

  private subscribe(
    api: LearningRequestApi,
    store: LearningRequestStore,
    started: { requestId: string; lastSeq: number },
  ) {
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

export function useLearningRequestActions(): Pick<
  LearningRequestContextValue,
  'startFollowup' | 'cancel' | 'startBook'
> | null {
  const context = useContext(LearningRequestContext);
  return context
    ? Object.freeze({
        startFollowup: context.startFollowup,
        cancel: context.cancel,
        startBook: context.startBook,
      })
    : null;
}
