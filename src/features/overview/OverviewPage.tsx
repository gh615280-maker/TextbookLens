import { useCallback, useEffect, useRef, useState } from 'react';
import { Link, useParams } from 'react-router-dom';

import { useMessage } from '../../app/LanguageProvider';
import type { LearningOverview } from '../../lib/generated/overview';
import { AnswerRenderer } from '../panels/AnswerRenderer';
import { CitationList } from '../panels/CitationList';
import {
  TauriBookConversationApi,
  type BookConversationApi,
  type BookConversationHistory,
  type BookConversationSummary,
} from '../history/conversation-api';
import {
  useLearningRequestActions,
  useLearningRequestSnapshot,
} from '../learning/LearningRequestProvider';
import type { LearningRequestView } from '../learning/learning-request-store';
import {
  TauriBookLearningPreparationApi,
  type BookLearningPreparationApi,
} from '../learning/book-api';
import { TauriLearningOverviewApi, type LearningOverviewApi } from './api';

const TERMINAL = new Set(['completed', 'failed', 'cancelled']);

interface OverviewPageProps {
  readonly overviewApi?: LearningOverviewApi;
  readonly historyApi?: BookConversationApi;
  readonly preparationApi?: BookLearningPreparationApi;
}

/**
 * A book-local surface: it reads only the compact local overview and explicit
 * book-history records.  Question context, provider selection and citations
 * remain exclusively backend-owned preparation data.
 */
export function OverviewPage({
  overviewApi,
  historyApi,
  preparationApi,
}: OverviewPageProps = {}) {
  const { bookId = '' } = useParams();
  return (
    <OverviewContent
      key={bookId}
      bookId={bookId}
      overviewApi={overviewApi}
      historyApi={historyApi}
      preparationApi={preparationApi}
    />
  );
}

function OverviewContent({
  bookId,
  overviewApi: suppliedOverviewApi,
  historyApi: suppliedHistoryApi,
  preparationApi: suppliedPreparationApi,
}: OverviewPageProps & { readonly bookId: string }) {
  const message = useMessage();
  const actions = useLearningRequestActions();
  const requestSnapshot = useLearningRequestSnapshot();
  const [overviewApi] = useState<LearningOverviewApi>(
    () => suppliedOverviewApi ?? new TauriLearningOverviewApi(),
  );
  const [historyApi] = useState<BookConversationApi>(
    () => suppliedHistoryApi ?? new TauriBookConversationApi(),
  );
  const [preparationApi] = useState<BookLearningPreparationApi>(
    () => suppliedPreparationApi ?? new TauriBookLearningPreparationApi(),
  );
  const [overview, setOverview] = useState<Readonly<LearningOverview> | null>(
    null,
  );
  const [overviewError, setOverviewError] = useState(false);
  const [summaries, setSummaries] = useState<
    readonly BookConversationSummary[]
  >([]);
  const [historyError, setHistoryError] = useState(false);
  const [historyLoading, setHistoryLoading] = useState(true);
  const [opened, setOpened] = useState<
    ReadonlyMap<string, BookConversationHistory>
  >(() => new Map());
  const [question, setQuestion] = useState('');
  const [questionBusy, setQuestionBusy] = useState(false);
  const [questionError, setQuestionError] = useState(false);
  const [confirmation, setConfirmation] = useState<{
    preparationId: string;
    provider: string;
    model: string;
    conversationId: string | null;
  } | null>(null);
  const [deleteCandidate, setDeleteCandidate] =
    useState<BookConversationSummary | null>(null);
  const [deleteError, setDeleteError] = useState(false);
  const [tombstones, setTombstones] = useState<ReadonlySet<string>>(
    () => new Set(),
  );
  const overviewGeneration = useRef(0);
  const historyGeneration = useRef(0);
  const seenTerminalRequests = useRef(new Set<string>());
  const tombstonesRef = useRef<ReadonlySet<string>>(new Set());
  const confirmationDialogRef = useRef<HTMLDialogElement>(null);
  const confirmationButtonRef = useRef<HTMLButtonElement>(null);
  const questionTriggerRef = useRef<HTMLButtonElement>(null);
  const deleteTriggerRef = useRef<HTMLButtonElement | null>(null);
  const deleteCancelRef = useRef<HTMLButtonElement>(null);

  const reloadHistory = useCallback(async () => {
    const generation = ++historyGeneration.current;
    setHistoryLoading(true);
    setHistoryError(false);
    try {
      const values = await historyApi.listBook(bookId);
      if (generation !== historyGeneration.current) return;
      setSummaries(
        values.filter((value) => !tombstonesRef.current.has(value.id)),
      );
    } catch {
      if (generation === historyGeneration.current) setHistoryError(true);
    } finally {
      if (generation === historyGeneration.current) setHistoryLoading(false);
    }
  }, [bookId, historyApi]);

  useEffect(() => {
    const generation = ++overviewGeneration.current;
    void overviewApi
      .get(bookId)
      .then((value) => {
        if (generation === overviewGeneration.current) setOverview(value);
      })
      .catch(() => {
        if (generation === overviewGeneration.current) setOverviewError(true);
      });
    return () => {
      // This only invalidates local rendering; it must never stop an AI request.
      if (generation === overviewGeneration.current)
        overviewGeneration.current += 1;
    };
  }, [bookId, overviewApi]);

  useEffect(() => {
    void Promise.resolve().then(reloadHistory);
  }, [reloadHistory]);

  useEffect(() => {
    confirmationButtonRef.current?.focus();
  }, [confirmation]);

  useEffect(() => {
    deleteCancelRef.current?.focus();
  }, [deleteCandidate]);

  const closeConfirmation = () => {
    if (!confirmation) return;
    void preparationApi.discard(confirmation.preparationId);
    setConfirmation(null);
    queueMicrotask(() => questionTriggerRef.current?.focus());
  };

  const closeDeleteDialog = () => {
    setDeleteCandidate(null);
    queueMicrotask(() => deleteTriggerRef.current?.focus());
  };

  const bookRequests = requestSnapshot.requests.filter(
    (request) => request.presentation?.bookId === bookId,
  );
  useEffect(() => {
    for (const request of bookRequests) {
      if (
        !TERMINAL.has(request.status) ||
        seenTerminalRequests.current.has(request.requestId)
      ) {
        continue;
      }
      seenTerminalRequests.current.add(request.requestId);
      void reloadHistory();
    }
  }, [bookRequests, reloadHistory]);

  const openHistory = async (summary: BookConversationSummary) => {
    if (tombstones.has(summary.id) || opened.has(summary.id)) return;
    const generation = historyGeneration.current;
    try {
      const result = await historyApi.getBook(bookId, summary.id);
      if (
        generation !== historyGeneration.current ||
        result.bookId !== bookId ||
        result.id !== summary.id ||
        tombstonesRef.current.has(summary.id)
      ) {
        return;
      }
      setOpened((current) => new Map(current).set(summary.id, result));
    } catch {
      setHistoryError(true);
    }
  };

  const submitQuestion = async (conversationId: string | null = null) => {
    const value = question.trim();
    if (!value || questionBusy || !actions) return;
    setQuestionBusy(true);
    setQuestionError(false);
    try {
      const summary = await preparationApi.prepare(
        conversationId
          ? { kind: 'continue', bookId, conversationId, question: value }
          : { kind: 'new', bookId, question: value },
      );
      const presentation = Object.freeze({
        action: conversationId ? 'continue' : 'ask',
        selectionLabel: 'Book question',
        provider: summary.providerDisplayName,
        model: summary.modelDisplayName,
        bookId,
      });
      if (summary.requiresBlockingConfirmation) {
        setConfirmation({
          preparationId: summary.preparationId,
          provider: summary.providerDisplayName,
          model: summary.modelDisplayName,
          conversationId,
        });
      } else {
        await actions.startBook(
          summary.preparationId,
          presentation,
          conversationId ?? undefined,
        );
        setQuestion('');
      }
    } catch {
      setQuestionError(true);
    } finally {
      setQuestionBusy(false);
    }
  };

  const confirmQuestion = async () => {
    if (!confirmation || !actions) return;
    setQuestionBusy(true);
    try {
      await preparationApi.authorize(confirmation.preparationId, 'allow');
      await actions.startBook(
        confirmation.preparationId,
        Object.freeze({
          action: confirmation.conversationId ? 'continue' : 'ask',
          selectionLabel: 'Book question',
          provider: confirmation.provider,
          model: confirmation.model,
          bookId,
        }),
        confirmation.conversationId ?? undefined,
      );
      setConfirmation(null);
      setQuestion('');
    } catch {
      setQuestionError(true);
    } finally {
      setQuestionBusy(false);
    }
  };

  const activeFor = (conversationId: string): LearningRequestView | undefined =>
    bookRequests.find(
      (request) =>
        !TERMINAL.has(request.status) &&
        (request.conversationId === conversationId ||
          request.targetConversationId === conversationId),
    );

  const deleteHistory = async () => {
    if (!deleteCandidate || activeFor(deleteCandidate.id)) return;
    setDeleteError(false);
    try {
      await historyApi.deleteBook(bookId, deleteCandidate.id);
      setTombstones((current) => {
        const next = new Set(current).add(deleteCandidate.id);
        tombstonesRef.current = next;
        return next;
      });
      setSummaries((current) =>
        current.filter((item) => item.id !== deleteCandidate.id),
      );
      setOpened((current) => {
        const next = new Map(current);
        next.delete(deleteCandidate.id);
        return next;
      });
      setDeleteCandidate(null);
    } catch {
      setDeleteError(true);
    }
  };

  return (
    <section
      aria-labelledby="overview-title"
      className="phase-page book-overview"
    >
      <h1 id="overview-title">{message('page.overview.title')}</h1>
      <p>{message('overview.localOnly')}</p>
      <div aria-live="polite" className="overview-status">
        {overviewError ? (
          <p role="alert">{message('overview.loadError')}</p>
        ) : null}
        {overview === null && !overviewError ? (
          <p>{message('loading.label')}</p>
        ) : null}
      </div>
      {overview ? (
        <OverviewDetails overview={overview} message={message} />
      ) : null}

      <section aria-labelledby="book-question-title">
        <h2 id="book-question-title">{message('overview.questions')}</h2>
        <label htmlFor="book-question">
          {message('overview.questionLabel')}
        </label>
        <textarea
          id="book-question"
          value={question}
          maxLength={16_384}
          disabled={questionBusy || !actions}
          onChange={(event) => setQuestion(event.target.value)}
        />
        <button
          ref={questionTriggerRef}
          type="button"
          disabled={!question.trim() || questionBusy || !actions}
          onClick={() => void submitQuestion()}
        >
          {questionBusy ? message('overview.preparing') : message('panel.send')}
        </button>
        {questionError ? (
          <p role="alert">
            {message('overview.questionError')}{' '}
            <Link to="/ai-services">{message('overview.aiServices')}</Link>
          </p>
        ) : null}
        {bookRequests.map((request) => (
          <LiveRequest
            key={request.requestId}
            request={request}
            onStop={() => void actions?.cancel(request.requestId)}
            message={message}
          />
        ))}
      </section>

      <section aria-labelledby="book-history-title">
        <h2 id="book-history-title">{message('overview.history')}</h2>
        {historyLoading ? <p>{message('loading.label')}</p> : null}
        {historyError ? (
          <p role="alert">{message('overview.historyError')}</p>
        ) : null}
        {!historyLoading && !historyError && summaries.length === 0 ? (
          <p>{message('overview.historyEmpty')}</p>
        ) : null}
        <ol className="book-history-list">
          {summaries.map((summary) => {
            const loaded = opened.get(summary.id);
            return (
              <li key={summary.id}>
                <button type="button" onClick={() => void openHistory(summary)}>
                  {summary.firstQuestionPreview}
                </button>{' '}
                <time dateTime={summary.updatedAt}>{summary.updatedAt}</time>{' '}
                <span>
                  {message('overview.messageCount', {
                    count: summary.messageCount,
                  })}
                </span>{' '}
                <button
                  type="button"
                  onClick={(event) => {
                    deleteTriggerRef.current = event.currentTarget;
                    setDeleteCandidate(summary);
                  }}
                >
                  {message('panel.delete')}
                </button>
                {loaded ? (
                  <BookHistory
                    history={loaded}
                    message={message}
                    onFollowup={() => void submitQuestion(summary.id)}
                    disabled={questionBusy || Boolean(activeFor(summary.id))}
                  />
                ) : null}
              </li>
            );
          })}
        </ol>
      </section>

      {confirmation ? (
        <dialog
          ref={confirmationDialogRef}
          open
          aria-labelledby="book-confirm-title"
          aria-describedby="book-confirm-details"
          aria-modal="true"
          onKeyDown={(event) => {
            if (event.key === 'Escape') {
              event.preventDefault();
              closeConfirmation();
            }
          }}
        >
          <h2 id="book-confirm-title">{message('learning.confirm.title')}</h2>
          <p id="book-confirm-details">
            {message('overview.confirmDetails', {
              provider: confirmation.provider,
              model: confirmation.model,
            })}
          </p>
          <button
            ref={confirmationButtonRef}
            type="button"
            onClick={() => void confirmQuestion()}
          >
            {message('learning.confirm.continue')}
          </button>
          <button type="button" onClick={closeConfirmation}>
            {message('learning.confirm.cancel')}
          </button>
        </dialog>
      ) : null}
      {deleteCandidate ? (
        <div
          aria-labelledby="book-delete-title"
          aria-modal="true"
          role="dialog"
          onKeyDown={(event) => {
            if (event.key === 'Escape') {
              event.preventDefault();
              closeDeleteDialog();
            }
          }}
        >
          <h2 id="book-delete-title">{message('overview.deleteTitle')}</h2>
          {activeFor(deleteCandidate.id) ? (
            <p role="status">{message('overview.deleteWait')}</p>
          ) : (
            <p>{message('overview.deleteConfirm')}</p>
          )}
          {deleteError ? (
            <p role="alert">{message('overview.deleteError')}</p>
          ) : null}
          <button
            type="button"
            disabled={Boolean(activeFor(deleteCandidate.id))}
            onClick={() => void deleteHistory()}
          >
            {message('panel.delete')}
          </button>
          <button
            ref={deleteCancelRef}
            type="button"
            onClick={closeDeleteDialog}
          >
            {message('learning.confirm.cancel')}
          </button>
        </div>
      ) : null}
    </section>
  );
}

function OverviewDetails({
  overview,
  message,
}: {
  overview: Readonly<LearningOverview>;
  message: ReturnType<typeof useMessage>;
}) {
  return (
    <>
      <section aria-labelledby="overview-contents">
        <h2 id="overview-contents">{message('overview.contents')}</h2>
        <ol>
          {overview.sections.map((section) => (
            <li key={section.id}>
              {section.title}:{' '}
              {message('overview.sectionStats', {
                text: section.localTextItemCount,
                notes: section.userNoteCount,
                conversations: section.completedConversationCount,
                exchanges: section.completedExchangeCount,
              })}
            </li>
          ))}
        </ol>
      </section>
      <section aria-labelledby="overview-coverage">
        <h2 id="overview-coverage">{message('overview.coverage')}</h2>
        <ul>
          {overview.sources.map((source) => (
            <li key={source.source}>
              <strong>{source.source}</strong>:{' '}
              {message('overview.sourceStats', {
                items: source.itemCount,
                sections: source.coveredSectionCount,
                pages: source.coveredPageCount,
              })}
              ;{' '}
              {source.quoteableAsTextbook
                ? message('overview.quoteable')
                : message('overview.notQuoteable')}
            </li>
          ))}
        </ul>
      </section>
      <section aria-labelledby="overview-activity">
        <h2 id="overview-activity">{message('overview.activity')}</h2>
        <p>
          {message('overview.activityStats', {
            notes: overview.activity.userNoteCount,
            conversations: overview.activity.completedConversationCount,
            exchanges: overview.activity.completedExchangeCount,
            citations: overview.activity.citationCount,
          })}
        </p>
        <p>
          {overview.teachingInstructionConfigured
            ? message('overview.teachingConfigured')
            : message('overview.teachingNotConfigured')}
        </p>
      </section>
    </>
  );
}

function LiveRequest({
  request,
  onStop,
  message,
}: {
  request: LearningRequestView;
  onStop(): void;
  message: ReturnType<typeof useMessage>;
}) {
  const running =
    request.status === 'preparing' || request.status === 'streaming';
  return (
    <article aria-live="polite">
      <h3>{message(`panel.status.${request.status}`)}</h3>
      {request.text ? <AnswerRenderer answer={request.text} /> : null}
      {request.usage ? (
        <p>
          {message('overview.usage', {
            input: request.usage.inputTokens ?? 0,
            output: request.usage.outputTokens ?? 0,
          })}
        </p>
      ) : null}
      {request.safeError ? (
        <p role="alert">{message('overview.questionError')}</p>
      ) : null}
      {running ? (
        <button type="button" onClick={onStop}>
          {message('panel.stop')}
        </button>
      ) : null}
    </article>
  );
}

function BookHistory({
  history,
  message,
  onFollowup,
  disabled,
}: {
  history: BookConversationHistory;
  message: ReturnType<typeof useMessage>;
  onFollowup(): void;
  disabled: boolean;
}) {
  return (
    <ol>
      {history.messages.map((item) => (
        <li key={item.id}>
          <strong>
            {item.role === 'user'
              ? message('overview.user')
              : message('overview.assistant')}
          </strong>
          {item.role === 'assistant' ? (
            <>
              <p>
                {message('overview.model', {
                  provider: item.providerId ?? '',
                  model: item.modelId ?? '',
                })}
              </p>
              <AnswerRenderer answer={item.content} />
              <CitationList
                citations={item.citations}
                emptyLabel={message('panel.noCitations')}
              />
            </>
          ) : (
            <p>{item.content}</p>
          )}
        </li>
      ))}
      <li>
        <button type="button" disabled={disabled} onClick={onFollowup}>
          {message('panel.followup')}
        </button>
      </li>
    </ol>
  );
}
