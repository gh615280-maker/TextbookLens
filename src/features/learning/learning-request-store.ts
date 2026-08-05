import type {
  LearningRequestEvent,
  LearningRequestSnapshot,
  LearningRequestStatus,
  LearningUsage,
  SafeLearningError,
} from '../../lib/generated/panel';

export interface LearningRequestView {
  readonly requestId: string;
  readonly conversationId: string | null;
  readonly status: LearningRequestStatus;
  readonly text: string;
  readonly usage: LearningUsage | null;
  readonly safeError: SafeLearningError | null;
  readonly lastSeq: number;
  readonly presentation: LearningRequestPresentation | null;
}

export interface LearningRequestPresentation {
  readonly action: string;
  readonly selectionLabel: string;
  readonly provider: string;
  readonly model: string;
}

export interface LearningRequestStoreSnapshot {
  readonly requests: readonly LearningRequestView[];
}

type Listener = () => void;

const terminal = new Set<LearningRequestStatus>([
  'completed',
  'failed',
  'cancelled',
]);

/** In-memory request state deliberately excludes preparations, credentials and captures. */
export class LearningRequestStore {
  private readonly requests = new Map<string, LearningRequestView>();
  private readonly listeners = new Set<Listener>();
  private currentSnapshot: LearningRequestStoreSnapshot = Object.freeze({
    requests: Object.freeze([]),
  });

  snapshot(): LearningRequestStoreSnapshot {
    return this.currentSnapshot;
  }

  subscribe(listener: Listener): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  get(requestId: string): LearningRequestView | undefined {
    return this.requests.get(requestId);
  }

  setPresentation(
    requestId: string,
    presentation: Readonly<LearningRequestPresentation>,
  ): boolean {
    const current = this.requests.get(requestId);
    if (!current) return false;
    this.requests.set(
      requestId,
      freezeView({
        ...current,
        presentation: Object.freeze({ ...presentation }),
      }),
    );
    this.emit();
    return true;
  }

  applySnapshot(snapshot: LearningRequestSnapshot): boolean {
    const next = freezeView(snapshot);
    const current = this.requests.get(next.requestId);
    if (current && current.lastSeq > next.lastSeq) return false;
    if (current && terminal.has(current.status)) return false;
    this.requests.set(next.requestId, next);
    this.emit();
    return true;
  }

  applyEvent(event: LearningRequestEvent): boolean {
    const current = this.requests.get(event.requestId);
    if (
      !current ||
      event.seq <= current.lastSeq ||
      terminal.has(current.status)
    ) {
      return false;
    }
    const next = applyEvent(current, event);
    if (!next) return false;
    this.requests.set(event.requestId, next);
    this.emit();
    return true;
  }

  private emit() {
    this.currentSnapshot = Object.freeze({
      requests: Object.freeze([...this.requests.values()]),
    });
    for (const listener of this.listeners) listener();
  }
}

function applyEvent(
  current: LearningRequestView,
  event: LearningRequestEvent,
): LearningRequestView | null {
  switch (event.event.type) {
    case 'preparing':
      return current.status === 'preparing'
        ? freezeView({ ...current, lastSeq: event.seq })
        : null;
    case 'text_delta':
      if (current.status !== 'preparing' && current.status !== 'streaming') {
        return null;
      }
      return freezeView({
        ...current,
        status: 'streaming',
        text: current.text + event.event.text,
        lastSeq: event.seq,
      });
    case 'usage':
      if (current.status !== 'preparing' && current.status !== 'streaming') {
        return null;
      }
      return freezeView({
        ...current,
        usage: {
          inputTokens: event.event.inputTokens,
          outputTokens: event.event.outputTokens,
        },
        lastSeq: event.seq,
      });
    case 'completed':
      if (current.status !== 'streaming') return null;
      return freezeView({
        ...current,
        conversationId: event.event.conversationId,
        status: 'completed',
        lastSeq: event.seq,
      });
    case 'failed':
      if (current.status !== 'preparing' && current.status !== 'streaming') {
        return null;
      }
      return freezeView({
        ...current,
        safeError: { code: event.event.safeError.code },
        status: 'failed',
        lastSeq: event.seq,
      });
    case 'cancelled':
      if (current.status !== 'preparing' && current.status !== 'streaming') {
        return null;
      }
      return freezeView({
        ...current,
        status: 'cancelled',
        lastSeq: event.seq,
      });
  }
}

function freezeView(
  value: LearningRequestSnapshot | LearningRequestView,
): LearningRequestView {
  return Object.freeze({
    requestId: value.requestId,
    conversationId: value.conversationId,
    status: value.status,
    text: value.text,
    usage: value.usage
      ? Object.freeze({
          inputTokens: value.usage.inputTokens,
          outputTokens: value.usage.outputTokens,
        })
      : null,
    safeError: value.safeError
      ? Object.freeze({ code: value.safeError.code })
      : null,
    lastSeq: value.lastSeq,
    presentation:
      'presentation' in value && value.presentation
        ? Object.freeze({ ...value.presentation })
        : null,
  });
}
