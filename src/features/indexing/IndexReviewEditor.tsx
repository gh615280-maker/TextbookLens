import { useEffect, useMemo, useState } from 'react';

import { useMessage } from '../../app/LanguageProvider';
import type { IndexPageReviewDto } from '../../lib/generated/indexing';
import { toOwnedArrayBuffer } from '../../lib/ipc';
import { TauriReaderApi, type ReaderApi } from '../reader/api';
import {
  releaseRenderedPdfPage,
  renderPdfPageLocally,
  type PdfPageRenderLimits,
} from './pdf-page-renderer';
import type { IndexingApi } from './api';

const REVIEW_RENDER_LIMITS: PdfPageRenderLimits = {
  maxDimension: 4096,
  maxDecodedPixels: 8_847_360,
  maxEncodedBytes: 4 * 1024 * 1024,
  maxTotalEncodedBytes: 4 * 1024 * 1024,
};

type ValueKind = 'text' | 'latex';

export function IndexReviewEditor({
  page,
  api,
  onChanged,
  readerApi: suppliedReaderApi,
  renderPage = renderPdfPageLocally,
}: {
  page: IndexPageReviewDto;
  api: Pick<
    IndexingApi,
    | 'saveCorrection'
    | 'resolveCorrectionConflict'
    | 'deleteCorrection'
    | 'retryPage'
  >;
  onChanged(): void;
  readerApi?: Pick<ReaderApi, 'readBookSource'>;
  renderPage?: typeof renderPdfPageLocally;
}) {
  const message = useMessage();
  const [readerApi] = useState<Pick<ReaderApi, 'readBookSource'>>(
    () => suppliedReaderApi ?? new TauriReaderApi(),
  );
  const [selectedBlockId, setSelectedBlockId] = useState<string | null>(
    page.blocks[0]?.id ?? null,
  );
  const [valueKind, setValueKind] = useState<ValueKind>('text');
  const [draft, setDraft] = useState(() =>
    valueFor(page, page.blocks[0]?.id ?? null, 'text'),
  );
  const [imageUrl, setImageUrl] = useState<string | null>(null);
  const [imageError, setImageError] = useState(false);
  const [busy, setBusy] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);
  const selected = useMemo(
    () => page.blocks.find((block) => block.id === selectedBlockId) ?? null,
    [page.blocks, selectedBlockId],
  );
  const correction = useMemo(
    () =>
      page.corrections.find(
        (item) =>
          item.targetBlockId === selected?.id && item.valueKind === valueKind,
      ) ?? null,
    [page.corrections, selected?.id, valueKind],
  );
  const original = selected
    ? ((valueKind === 'text' ? selected.plainText : selected.latex) ?? '')
    : '';

  useEffect(() => {
    let active = true;
    let url: string | null = null;
    let sourceBytes: Uint8Array | undefined;
    let source: ArrayBuffer | undefined;
    let rendered: Awaited<ReturnType<typeof renderPdfPageLocally>> | undefined;
    void (async () => {
      await Promise.resolve();
      if (!active) return;
      setImageError(false);
      setImageUrl(null);
      try {
        const localSourceBytes = await readerApi
          .readBookSource(page.bookId)
          .then((value) =>
            value instanceof Uint8Array ? value : new Uint8Array(value),
          );
        sourceBytes = localSourceBytes;
        source = toOwnedArrayBuffer(localSourceBytes);
        localSourceBytes.fill(0);
        sourceBytes = undefined;
        rendered = await renderPage(
          source,
          page.pageNumber,
          REVIEW_RENDER_LIMITS,
        );
        if (!active) return;
        const imageCopy = Uint8Array.from(rendered.bytes);
        url = URL.createObjectURL(
          new Blob([imageCopy.buffer as ArrayBuffer], {
            type: rendered.mimeType,
          }),
        );
        imageCopy.fill(0);
        setImageUrl(url);
      } catch {
        if (active) setImageError(true);
      } finally {
        sourceBytes?.fill(0);
        wipeSource(source);
        if (rendered) releaseRenderedPdfPage(rendered);
      }
    })();
    return () => {
      active = false;
      sourceBytes?.fill(0);
      wipeSource(source);
      if (rendered) releaseRenderedPdfPage(rendered);
      if (url) URL.revokeObjectURL(url);
    };
  }, [page.bookId, page.pageNumber, readerApi, renderPage]);

  async function save() {
    if (!selected || busy) return;
    setBusy(true);
    setActionError(null);
    try {
      await api.saveCorrection({
        bookId: page.bookId,
        pageId: page.id,
        targetBlockId: selected.id,
        targetContentVersion: page.contentVersion,
        valueKind,
        originalValueSha256: await sha256(original),
        correctedValue: draft,
        expectedRevision: correction?.revision ?? 0,
      });
      onChanged();
    } catch {
      setActionError(message('indexQuality.review.saveError'));
    } finally {
      setBusy(false);
    }
  }

  async function resolve(decision: 'keep' | 'accept' | 'compare') {
    if (!correction || busy) return;
    setBusy(true);
    setActionError(null);
    try {
      await api.resolveCorrectionConflict({
        bookId: page.bookId,
        pageId: page.id,
        correctionId: correction.id,
        targetContentVersion: page.contentVersion,
        currentValueSha256: await sha256(original),
        expectedRevision: correction.revision,
        decision,
        comparedCorrectedValue: decision === 'compare' ? draft : null,
      });
      onChanged();
    } catch {
      setActionError(message('indexQuality.review.resolveError'));
    } finally {
      setBusy(false);
    }
  }

  async function remove() {
    if (!correction || busy) return;
    setBusy(true);
    setActionError(null);
    try {
      await api.deleteCorrection({
        bookId: page.bookId,
        pageId: page.id,
        correctionId: correction.id,
        targetContentVersion: page.contentVersion,
        currentValueSha256: await sha256(original),
        expectedRevision: correction.revision,
      });
      onChanged();
    } catch {
      setActionError(message('indexQuality.review.deleteError'));
    } finally {
      setBusy(false);
    }
  }

  async function retry() {
    if (busy) return;
    setBusy(true);
    setActionError(null);
    try {
      await api.retryPage(page.id, page.updatedAt);
      onChanged();
    } catch {
      setActionError(message('indexQuality.review.retryError'));
    } finally {
      setBusy(false);
    }
  }

  return (
    <section aria-labelledby="index-review-title">
      <h2 id="index-review-title">
        {message('indexQuality.review.title', { page: page.pageNumber })}
      </h2>
      {page.safeError ? <p role="alert">{page.safeError.message}</p> : null}
      <div className="index-review-editor__columns">
        <div>
          <h3>{message('indexQuality.review.original')}</h3>
          {imageUrl ? (
            <img
              alt={message('indexQuality.review.originalAlt', {
                page: page.pageNumber,
              })}
              src={imageUrl}
            />
          ) : null}
          {imageError ? (
            <p role="alert">
              {message('indexQuality.review.previewUnavailable')}
            </p>
          ) : null}
          {!imageUrl && !imageError ? (
            <p aria-live="polite">{message('indexQuality.review.rendering')}</p>
          ) : null}
        </div>
        <div>
          <h3>{message('indexQuality.review.editor')}</h3>
          <label>
            {message('indexQuality.review.block')}
            <select
              value={selectedBlockId ?? ''}
              onChange={(event) => {
                const next = event.currentTarget.value || null;
                setSelectedBlockId(next);
                setDraft(valueFor(page, next, valueKind));
              }}
            >
              {page.blocks.map((block) => (
                <option key={block.id} value={block.id}>
                  {message('indexQuality.review.blockOption', {
                    number: block.ordinal + 1,
                    kind: message(`indexQuality.block.${block.kind}`),
                  })}
                </option>
              ))}
            </select>
          </label>
          <label>
            {message('indexQuality.review.valueType')}
            <select
              value={valueKind}
              onChange={(event) => {
                const next = event.currentTarget.value as ValueKind;
                setValueKind(next);
                setDraft(valueFor(page, selectedBlockId, next));
              }}
            >
              <option value="text">
                {message('indexQuality.review.transcription')}
              </option>
              <option value="latex">
                {message('indexQuality.review.latex')}
              </option>
            </select>
          </label>
          <label>
            {message('indexQuality.review.editable')}
            <textarea
              value={draft}
              onChange={(event) => setDraft(event.currentTarget.value)}
            />
          </label>
          <p aria-live="polite">
            {busy
              ? message('indexQuality.review.saving')
              : correction?.conflictState === 'conflict'
                ? message('indexQuality.review.conflict')
                : ''}
          </p>
          {actionError ? <p role="alert">{actionError}</p> : null}
          <button
            disabled={busy}
            type="button"
            onClick={() => setDraft(original)}
          >
            {message('indexQuality.review.keepLocal')}
          </button>
          <button
            disabled={busy || !selected}
            type="button"
            onClick={() => void save()}
          >
            {message('indexQuality.review.save')}
          </button>
          <button disabled={busy} type="button" onClick={() => void retry()}>
            {message('indexQuality.review.retry')}
          </button>
          {correction?.conflictState === 'conflict' ? (
            <>
              <button
                disabled={busy}
                type="button"
                onClick={() => void resolve('keep')}
              >
                {message('indexQuality.review.keepCorrection')}
              </button>
              <button
                disabled={busy}
                type="button"
                onClick={() => void resolve('accept')}
              >
                {message('indexQuality.review.acceptCurrent')}
              </button>
              <button
                disabled={busy}
                type="button"
                onClick={() => void resolve('compare')}
              >
                {message('indexQuality.review.compare')}
              </button>
            </>
          ) : null}
          {correction ? (
            <button disabled={busy} type="button" onClick={() => void remove()}>
              {message('indexQuality.review.delete')}
            </button>
          ) : null}
        </div>
      </div>
    </section>
  );
}

async function sha256(value: string): Promise<string> {
  const digest = await crypto.subtle.digest(
    'SHA-256',
    new TextEncoder().encode(value),
  );
  return [...new Uint8Array(digest)]
    .map((part) => part.toString(16).padStart(2, '0'))
    .join('');
}

function valueFor(
  page: IndexPageReviewDto,
  blockId: string | null,
  valueKind: ValueKind,
): string {
  const correction = page.corrections.find(
    (item) => item.targetBlockId === blockId && item.valueKind === valueKind,
  );
  if (correction) return correction.correctedValue;
  const block = page.blocks.find((item) => item.id === blockId);
  return (valueKind === 'text' ? block?.plainText : block?.latex) ?? '';
}

function wipeSource(source: ArrayBuffer | undefined): void {
  if (!source || source.byteLength === 0) return;
  new Uint8Array(source).fill(0);
}
