import { useEffect, useState } from 'react';
import { useNavigate, useParams } from 'react-router-dom';
import { useMessage } from '../../app/LanguageProvider';
import type { ProviderProfileSummary } from '../../lib/generated/provider';
import { TauriProviderApi } from '../providers/api';
import { TauriExtractionApi, type ExtractionSummary } from './api';

export function ExtractionStartPage() {
  const { bookId } = useParams();
  const navigate = useNavigate();
  const message = useMessage();
  const [api] = useState(() => new TauriExtractionApi());
  const [providers] = useState(() => new TauriProviderApi());
  const [profile, setProfile] = useState<ProviderProfileSummary | null>(null);
  const [summary, setSummary] = useState<ExtractionSummary | null>(null);
  const [state, setState] = useState<
    'loading' | 'ready' | 'running' | 'failed'
  >('loading');
  useEffect(() => {
    if (!bookId) return;
    let active = true;
    void Promise.all([
      api.get(bookId),
      providers.getSettings(),
      providers.listProfiles('text_learning'),
    ])
      .then(([current, settings, profiles]) => {
        if (!active) return;
        setSummary(current);
        setProfile(
          profiles.find(
            (p) =>
              p.kind === 'kimi' &&
              p.id ===
                (settings.defaultLearningProfileId ??
                  settings.activeProviderProfileId) &&
              p.credentialStatus === 'available',
          ) ??
            profiles.find(
              (p) => p.kind === 'kimi' && p.credentialStatus === 'available',
            ) ??
            null,
        );
        setState('ready');
      })
      .catch(() => active && setState('failed'));
    return () => {
      active = false;
    };
  }, [api, bookId, providers]);
  async function start() {
    if (!bookId || !profile) return;
    setState('running');
    try {
      const result = await api.prepare(bookId, profile.id);
      setSummary(result);
      setState('ready');
    } catch {
      setState('failed');
    }
  }
  if (!bookId) return null;
  return (
    <section aria-labelledby="extraction-title">
      <h1 id="extraction-title">{message('extraction.title')}</h1>
      <p>{message('extraction.description')}</p>
      {state === 'loading' ? (
        <p role="status">{message('extraction.loading')}</p>
      ) : null}
      {summary?.status === 'ready' ? (
        <p role="status">
          {message('extraction.ready', { chunks: summary.chunkCount })}
        </p>
      ) : null}
      {state === 'failed' ? (
        <p role="alert">{message('extraction.failed')}</p>
      ) : null}
      {!profile && state !== 'loading' ? (
        <button type="button" onClick={() => navigate('/ai-services')}>
          {message('extraction.configure')}
        </button>
      ) : null}
      {profile && summary?.status !== 'ready' ? (
        <button
          type="button"
          disabled={state === 'running'}
          onClick={() => void start()}
        >
          {state === 'running'
            ? message('extraction.running')
            : message('extraction.start')}
        </button>
      ) : null}
      <button
        type="button"
        onClick={() =>
          navigate(`/books/${encodeURIComponent(bookId)}/index-advanced`)
        }
      >
        {message('extraction.advanced')}
      </button>
      <button
        type="button"
        onClick={() => navigate(`/books/${encodeURIComponent(bookId)}/read`)}
      >
        {message('indexStart.continueReading')}
      </button>
    </section>
  );
}
