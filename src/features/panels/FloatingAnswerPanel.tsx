import { useEffect, useRef, useState } from 'react';

import { useMessage } from '../../app/LanguageProvider';
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
  onMoveKeyDown(event: React.KeyboardEvent<HTMLDivElement>): void;
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
  onMoveKeyDown,
  onHide,
  onCollapse,
  onStop,
  onFollowup,
  onRetry,
  onDelete,
  deleteDisabled = false,
  deleteError = false,
}: FloatingAnswerPanelProps) {
  const message = useMessage();
  const [announcement, setAnnouncement] = useState('');
  const announcedTerminal = useRef<string | null>(null);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const active = !terminalStatuses.has(request.status);
  const title = request.presentation?.selectionLabel ?? message('panel.title');
  const status = request.status;
  const statusLabel = message(`panel.status.${status}`);
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
      setAnnouncement(
        message('panel.announcement.status', {
          status: message(`panel.status.${request.status}`),
        }),
      );
      return;
    }
    if (request.status === 'streaming' && !timer.current) {
      timer.current = setTimeout(() => {
        timer.current = null;
        setAnnouncement(message('panel.announcement.updating'));
      }, 700);
    }
  }, [message, request.status, request.text]);
  return (
    <>
      <section
        aria-label={message('panel.answer')}
        className="floating-answer-panel"
      >
        <PanelTitleBar
          collapsed={collapsed}
          status={statusLabel}
          title={title}
          moveLabel={message('panel.move')}
          collapseLabel={message('panel.collapse')}
          expandLabel={message('panel.expand')}
          hideLabel={message('panel.hide')}
          onCollapse={onCollapse}
          onDragStart={onDragStart}
          onMoveKeyDown={onMoveKeyDown}
          onHide={onHide}
        />
        {!collapsed ? (
          <div className="floating-panel-body">
            {history ? <HistoryMessages history={history} /> : null}
            {!history || request.status !== 'completed' ? (
              <>
                <p>
                  <strong>{message('panel.action')}:</strong>{' '}
                  {request.presentation?.action ?? 'learning'}
                </p>
                <AnswerRenderer answer={request.text} />
                <h3>{message('panel.citations')}</h3>
                <CitationList
                  citations={citations}
                  emptyLabel={message('panel.noCitations')}
                />
              </>
            ) : null}
            {request.safeError ? (
              <p role="alert">
                {message('panel.requestFailed', {
                  code: request.safeError.code,
                })}
              </p>
            ) : null}
            {deleteError ? (
              <p role="alert">{message('panel.deleteFailed')}</p>
            ) : null}
            <div className="floating-panel-actions">
              {active ? (
                <button onClick={onStop} type="button">
                  {message('panel.stop')}
                </button>
              ) : null}
              <button disabled={!onRetry} onClick={onRetry} type="button">
                {message('panel.retry')}
              </button>
              <button
                disabled={!onDelete || deleteDisabled || active}
                onClick={onDelete}
                type="button"
              >
                {message('panel.delete')}
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
  const message = useMessage();
  return (
    <>
      <p className="floating-history-selection">
        <strong>{message('panel.originalSelection')}:</strong>{' '}
        {history.selectedText ?? message('panel.visualRegion')}
      </p>
      <ol className="floating-history-messages">
        {history.messages.map((historyMessage) => (
          <li key={historyMessage.id} data-message-role={historyMessage.role}>
            {historyMessage.role === 'user' ? (
              <p className="floating-history-question">
                {historyMessage.content}
              </p>
            ) : (
              <>
                <p className="floating-history-provider">
                  {message('panel.providerModel', {
                    provider: historyMessage.providerId ?? '',
                    model: historyMessage.modelId ?? '',
                  })}
                </p>
                <AnswerRenderer answer={historyMessage.content} />
                <CitationList
                  citations={historyMessage.citations}
                  emptyLabel={message('panel.noCitations')}
                />
              </>
            )}
          </li>
        ))}
      </ol>
    </>
  );
}
