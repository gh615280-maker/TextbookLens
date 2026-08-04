import { useEffect, useState } from 'react';
import { useNavigate, useParams } from 'react-router-dom';

import { useMessage } from '../../app/LanguageProvider';
import type { ProviderProfileSummary } from '../../lib/generated/provider';
import { inspectLocalPdfPageQuality } from '../import/parsers/pdf-parser';
import { TauriProviderApi, type ProviderApi } from '../providers/api';
import { TauriReaderApi, type ReaderApi } from '../reader/api';
import {
  TauriIndexingApi,
  type ConfirmIndexOperationRequest,
  type IndexingApi,
} from './api';
import { IndexStartConfirmation } from './IndexStartConfirmation';
import type { LocalPdfPageQualityDto } from './indexing-contract';

type ReaderStartApi = Pick<ReaderApi, 'getReaderBootstrap' | 'readBookSource'>;
type ProviderStartApi = Pick<
  ProviderApi,
  'getSettings' | 'listProfiles' | 'updateConsent'
>;
type QualityInspector = (
  source: ArrayBuffer,
  signal: AbortSignal,
) => Promise<LocalPdfPageQualityDto[]>;

export interface IndexStartPageProps {
  readerApi?: ReaderStartApi;
  providerApi?: ProviderStartApi;
  indexingApi?: Pick<IndexingApi, 'confirmOperation' | 'createRun'>;
  inspectQuality?: QualityInspector;
}

type FailureReason =
  'not_ready_pdf' | 'reliable_only' | 'profile_missing' | 'local_failure';

type StartState =
  | { kind: 'loading' }
  | { kind: 'failure'; reason: FailureReason }
  | {
      kind: 'ready';
      request: ConfirmIndexOperationRequest;
      profile: ProviderProfileSummary;
    };

export function IndexStartPage({
  readerApi,
  providerApi,
  indexingApi,
  inspectQuality,
}: IndexStartPageProps = {}) {
  const { bookId } = useParams();
  const navigate = useNavigate();
  const message = useMessage();
  const [reader] = useState<ReaderStartApi>(
    () => readerApi ?? new TauriReaderApi(),
  );
  const [providers] = useState<ProviderStartApi>(
    () => providerApi ?? new TauriProviderApi(),
  );
  const [indexing] = useState<
    Pick<IndexingApi, 'confirmOperation' | 'createRun'>
  >(() => indexingApi ?? new TauriIndexingApi());
  const [qualityInspector] = useState<QualityInspector>(
    () => inspectQuality ?? inspectLocalPdfPageQuality,
  );
  const [state, setState] = useState<StartState>({ kind: 'loading' });

  useEffect(() => {
    if (!bookId) return;
    const resolvedBookId = bookId;

    let active = true;
    const abortController = new AbortController();
    let receivedSource: Uint8Array | ArrayBuffer | undefined;
    let localSource: ArrayBuffer | undefined;

    async function prepare() {
      try {
        const bootstrap = await reader.getReaderBootstrap(resolvedBookId);
        if (!active) return;
        if (
          bootstrap.book.importStatus !== 'ready' ||
          bootstrap.book.format !== 'pdf'
        ) {
          setState({ kind: 'failure', reason: 'not_ready_pdf' });
          return;
        }

        let sourceSha256: string;
        let qualities: LocalPdfPageQualityDto[];
        try {
          receivedSource = await reader.readBookSource(resolvedBookId);
          if (!active) return;
          localSource = copySource(receivedSource);
          wipeSource(receivedSource);
          receivedSource = undefined;
          if (!active) return;
          sourceSha256 = await sha256(localSource);
          if (!active) return;
          qualities = await qualityInspector(
            localSource,
            abortController.signal,
          );
        } finally {
          wipeSource(receivedSource);
          wipeSource(localSource);
          receivedSource = undefined;
          localSource = undefined;
        }

        if (!active) return;
        const abnormalPages = qualities.filter(
          (quality) => quality.qualityReason !== 'reliable_text',
        );
        if (abnormalPages.length === 0) {
          setState({ kind: 'failure', reason: 'reliable_only' });
          return;
        }

        const [settings, compatibleProfiles] = await Promise.all([
          providers.getSettings(),
          providers.listProfiles('structured_page_analysis'),
        ]);
        if (!active) return;
        const profile = compatibleProfiles.find(
          (candidate) =>
            candidate.id === settings.defaultVisionProfileId &&
            candidate.credentialStatus === 'available' &&
            candidate.validatedAt !== null,
        );
        if (!profile) {
          setState({ kind: 'failure', reason: 'profile_missing' });
          return;
        }

        setState({
          kind: 'ready',
          profile,
          request: {
            runId: null,
            bookId: resolvedBookId,
            sourceSha256,
            providerProfileId: profile.id,
            pages: abnormalPages.map((quality) => ({
              pageNumber: quality.pageNumber,
              qualityReason: quality.qualityReason,
              localTextSha256: null,
            })),
          },
        });
      } catch (error) {
        if (
          active &&
          !(error instanceof DOMException && error.name === 'AbortError')
        ) {
          setState({ kind: 'failure', reason: 'local_failure' });
        }
      }
    }

    void prepare();
    return () => {
      active = false;
      abortController.abort();
      wipeSource(receivedSource);
      wipeSource(localSource);
    };
  }, [bookId, providers, qualityInspector, reader]);

  const readRoute = bookId
    ? `/books/${encodeURIComponent(bookId)}/read`
    : '/library';

  return (
    <section aria-labelledby="index-start-title">
      <h1 id="index-start-title">{message('indexStart.title')}</h1>
      <p>{message('indexStart.description')}</p>
      {state.kind === 'loading' ? (
        <p aria-live="polite" role="status">
          {message('indexStart.preparing')}
        </p>
      ) : null}
      {state.kind === 'failure' ? (
        <div role="alert">
          <p>{message(failureMessage(state.reason))}</p>
          <button
            type="button"
            onClick={() =>
              navigate(
                state.reason === 'profile_missing' ? '/ai-services' : readRoute,
              )
            }
          >
            {state.reason === 'profile_missing'
              ? message('indexStart.configure')
              : message('indexStart.continueReading')}
          </button>
        </div>
      ) : null}
      {state.kind === 'ready' ? (
        <IndexStartConfirmation
          api={indexing}
          modelName={state.profile.modelId}
          profileName={state.profile.displayName}
          request={state.request}
          onReject={() => navigate(`${readRoute}?index=local-only`)}
          onSetProfileNoPrompt={(profileId) =>
            providers.updateConsent(profileId, 'ai_index', 'skip_prompt')
          }
          onStarted={(runId) =>
            navigate(
              `/books/${encodeURIComponent(state.request.bookId)}/index-quality/${encodeURIComponent(runId)}`,
            )
          }
        />
      ) : null}
    </section>
  );
}

function failureMessage(reason: FailureReason) {
  switch (reason) {
    case 'not_ready_pdf':
      return 'indexStart.notReadyPdf' as const;
    case 'reliable_only':
      return 'indexStart.reliableOnly' as const;
    case 'profile_missing':
      return 'indexStart.profileMissing' as const;
    case 'local_failure':
      return 'indexStart.localFailure' as const;
  }
}

function copySource(source: Uint8Array | ArrayBuffer): ArrayBuffer {
  if (source instanceof ArrayBuffer) return source.slice(0);
  return Uint8Array.from(source).buffer;
}

function wipeSource(source: Uint8Array | ArrayBuffer | undefined): void {
  if (!source) return;
  if (source instanceof ArrayBuffer) {
    if (source.byteLength === 0) return;
    new Uint8Array(source).fill(0);
    return;
  }
  source.fill(0);
}

async function sha256(source: ArrayBuffer): Promise<string> {
  const digest = await crypto.subtle.digest('SHA-256', source);
  return [...new Uint8Array(digest)]
    .map((byte) => byte.toString(16).padStart(2, '0'))
    .join('');
}
