import { useEffect, useRef, useState } from 'react';

import type { LearningRequestView } from '../learning/learning-request-store';
import type { Citation } from '../../lib/generated/conversation';
import type { ConversationHistory } from '../history/conversation-api';
import { AnswerRenderer } from './AnswerRenderer';
import { CitationList } from './CitationList';
import { FollowupComposer } from './FollowupComposer';
import { PanelTitleBar } from './PanelTitleBar';

interface FloatingAnswerPanelProps {
  readonly request: LearningRequestView;
  readonly collapsed: boolean;
  readonly citations?: readonly Pick<Citation, 'id' | 'label' | 'quoteable'>[];
  readonly history?: ConversationHistory;
  onDragStart(event: React.PointerEvent<HTMLDivElement>): void;
  onHide(): void;
  onCollapse(): void;
  onStop(): void;
  onFollowup(question: string): void;
  /** Undefined means no safe preparation/history retry boundary is available. */
  onRetry?: () => void;
  onDelete?: () => void;
  deleteDisabled?: boolean;
  deleteError?: boolean;
}

const terminalStatuses = new Set(['completed', 'failed', 'cancelled']);

export function FloatingAnswerPanel({
  request,
  collapsed,
  citations = [],
  history,
  onDragStart,
  onHide,
  onCollapse,
  onStop,
  onFollowup,
  onRetry,
  onDelete,
  deleteDisabled = false,
  deleteError = false,
}: FloatingAnswerPanelProps) {
  const [announcement, setAnnouncement] = useState('');
  const announcedTerminal = useRef<string | null>(null);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const active = !terminalStatuses.has(request.status);
  const title = request.presentation?.selectionLabel ?? 'Learning request';
  const status = request.status;
  useEffect(
    () => () => {
      if (timer.current) clearTimeout(timer.current);
    },
    [],
  );
  useEffect(() => {
    if (terminalStatuses.has(request.status)) {
      if (announcedTerminal.current === request.status) return;
      announcedTerminal.current = request.status;
      setAnnouncement(`Learning request ${request.status}`);
      return;
    }
    if (request.status === 'streaming' && !timer.current) {
      timer.current = setTimeout(() => {
        timer.current = null;
        setAnnouncement('Learning answer is updating');
      }, 700);
    }
  }, [request.status, request.text]);
  return (
    <>
      <section aria-label="Learning answer" className="floating-answer-panel">
        <PanelTitleBar
          collapsed={collapsed}
          status={status}
          title={title}
          onCollapse={onCollapse}
          onDragStart={onDragStart}
          onHide={onHide}
        />
        {!collapsed ? (
          <div className="floating-panel-body">
            {history ? <HistoryMessages history={history} /> : null}
            {!history || request.status !== 'completed' ? (
              <>
                <p>
                  <strong>Action:</strong>{' '}
                  {request.presentation?.action ?? 'learning'}
                </p>
                <AnswerRenderer answer={request.text} />
                <h3>Citations</h3>
                <CitationList
                  citations={citations}
                  emptyLabel="No verified citations available."
                />
              </>
            ) : null}
            {request.safeError ? (
              <p role="alert">Request failed: {request.safeError.code}</p>
            ) : null}
            {deleteError ? (
              <p role="alert">
                The conversation could not be deleted. Nothing was removed.
              </p>
            ) : null}
            <div className="floating-panel-actions">
              {active ? (
                <button onClick={onStop} type="button">
                  Stop
                </button>
              ) : null}
              <button disabled={!onRetry} onClick={onRetry} type="button">
                Retry
              </button>
              <button
                disabled={!onDelete || deleteDisabled || active}
                onClick={onDelete}
                type="button"
              >
                Delete
              </button>
            </div>
            <FollowupComposer
              disabled={!request.conversationId || active}
              providerChangeNotice={null}
              onSubmit={onFollowup}
            />
          </div>
        ) : null}
      </section>
      <p aria-atomic="true" aria-live="polite" className="floating-panel-live">
        {announcement}
      </p>
    </>
  );
}

function HistoryMessages({ history }: { history: ConversationHistory }) {
  return (
    <>
      <p className="floating-history-selection">
        <strong>Original selection:</strong>{' '}
        {history.selectedText ?? 'Visual region'}
      </p>
      <ol className="floating-history-messages">
        {history.messages.map((message) => (
          <li key={message.id} data-message-role={message.role}>
            {message.role === 'user' ? (
              <p className="floating-history-question">{message.content}</p>
            ) : (
              <>
                <p className="floating-history-provider">
                  Provider {message.providerId} · Model {message.modelId}
                </p>
                <AnswerRenderer answer={message.content} />
                <CitationList
                  citations={message.citations}
                  emptyLabel="No verified citations available."
                />
              </>
            )}
          </li>
        ))}
      </ol>
    </>
  );
}
