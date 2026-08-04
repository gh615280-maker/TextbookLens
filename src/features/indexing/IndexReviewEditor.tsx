import { useEffect, useMemo, useState } from 'react';

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
        if (source) new Uint8Array(source).fill(0);
        if (rendered) releaseRenderedPdfPage(rendered);
      }
    })();
    return () => {
      active = false;
      sourceBytes?.fill(0);
      if (source) new Uint8Array(source).fill(0);
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
      setActionError(
        'The correction could not be saved. Review the current page and try again.',
      );
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
      setActionError(
        'The correction conflict could not be resolved. Reload the page and try again.',
      );
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
      setActionError(
        'The correction could not be deleted. Reload the page and try again.',
      );
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
      setActionError(
        'The page could not be retried. Review the durable run state and try again.',
      );
    } finally {
      setBusy(false);
    }
  }

  return (
    <section aria-labelledby="index-review-title">
      <h2 id="index-review-title">Review page {page.pageNumber}</h2>
      {page.safeError ? <p role="alert">{page.safeError.message}</p> : null}
      <div className="index-review-editor__columns">
        <div>
          <h3>Local original page</h3>
          {imageUrl ? (
            <img
              alt={`Local original page ${page.pageNumber}`}
              src={imageUrl}
            />
          ) : null}
          {imageError ? (
            <p role="alert">The local page preview is unavailable.</p>
          ) : null}
          {!imageUrl && !imageError ? (
            <p aria-live="polite">Rendering local page preview…</p>
          ) : null}
        </div>
        <div>
          <h3>Editable transcription and LaTeX</h3>
          <label>
            Content block
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
                  Block {block.ordinal + 1}: {block.kind}
                </option>
              ))}
            </select>
          </label>
          <label>
            Value type
            <select
              value={valueKind}
              onChange={(event) => {
                const next = event.currentTarget.value as ValueKind;
                setValueKind(next);
                setDraft(valueFor(page, selectedBlockId, next));
              }}
            >
              <option value="text">Transcription</option>
              <option value="latex">LaTeX</option>
            </select>
          </label>
          <label>
            Editable value
            <textarea
              value={draft}
              onChange={(event) => setDraft(event.currentTarget.value)}
            />
          </label>
          <p aria-live="polite">
            {busy
              ? 'Saving review change…'
              : correction?.conflictState === 'conflict'
                ? 'Correction conflict needs a decision.'
                : ''}
          </p>
          {actionError ? <p role="alert">{actionError}</p> : null}
          <button
            disabled={busy}
            type="button"
            onClick={() => setDraft(original)}
          >
            Keep local page value
          </button>
          <button
            disabled={busy || !selected}
            type="button"
            onClick={() => void save()}
          >
            Save correction
          </button>
          <button disabled={busy} type="button" onClick={() => void retry()}>
            Retry page
          </button>
          {correction?.conflictState === 'conflict' ? (
            <>
              <button
                disabled={busy}
                type="button"
                onClick={() => void resolve('keep')}
              >
                Keep correction
              </button>
              <button
                disabled={busy}
                type="button"
                onClick={() => void resolve('accept')}
              >
                Accept current value
              </button>
              <button
                disabled={busy}
                type="button"
                onClick={() => void resolve('compare')}
              >
                Resolve with compared value
              </button>
            </>
          ) : null}
          {correction ? (
            <button disabled={busy} type="button" onClick={() => void remove()}>
              Delete correction
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
