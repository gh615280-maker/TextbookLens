import { toOwnedArrayBuffer } from '../../lib/ipc';
import {
  PdfPageRenderError,
  releaseRenderedPdfPage,
  renderPdfPageLocally,
  type PdfPageRenderLimits,
} from './pdf-page-renderer';
import type { RenderedPdfPageDto } from './indexing-contract';
import {
  TauriIndexingApi,
  toSubmission,
  type IndexingApi,
  type IndexingEvent,
  type RenderClaim,
  type RenderClaimBatch,
} from './api';

const EMPTY_POLL_DELAY_MS = 250;
const ERROR_POLL_DELAY_MS = 1_000;
const CLAIM_TIMEOUT_MS = 10_000;
const SOURCE_TIMEOUT_MS = 30_000;
const RENDER_TIMEOUT_MS = 60_000;
const SUBMIT_TIMEOUT_MS = 120_000;

export type LocalPageRenderer = (
  source: ArrayBuffer,
  pageNumber: number,
  limits: PdfPageRenderLimits,
) => Promise<RenderedPdfPageDto>;

export class IndexingCoordinator {
  #controller: AbortController | null = null;
  #loop: Promise<void> | null = null;
  #generation = 0;
  #listeners = new Set<(event: IndexingEvent) => void>();

  constructor(
    private readonly api: IndexingApi = new TauriIndexingApi(),
    private readonly renderPage: LocalPageRenderer = renderPdfPageLocally,
  ) {}

  get running(): boolean {
    return this.#controller !== null && !this.#controller.signal.aborted;
  }

  start(): void {
    if (this.running) return;
    const controller = new AbortController();
    const generation = ++this.#generation;
    this.#controller = controller;
    this.#loop = this.#run(controller.signal).finally(() => {
      if (this.#generation === generation) {
        this.#controller = null;
        this.#loop = null;
      }
    });
  }

  async stop(): Promise<void> {
    const loop = this.#loop;
    this.#controller?.abort();
    if (loop) await loop;
  }

  subscribe(listener: (event: IndexingEvent) => void): () => void {
    this.#listeners.add(listener);
    return () => this.#listeners.delete(listener);
  }

  async #run(signal: AbortSignal): Promise<void> {
    while (!signal.aborted) {
      try {
        const batch = await withTimeout(
          this.api.claimRenderBatch(),
          CLAIM_TIMEOUT_MS,
          signal,
        );
        assertSafeBatch(batch);
        if (batch.claims.length === 0) {
          await abortableDelay(EMPTY_POLL_DELAY_MS, signal);
          continue;
        }
        await this.#processBatch(batch, signal);
      } catch {
        if (signal.aborted) return;
        await abortableDelay(ERROR_POLL_DELAY_MS, signal);
      }
    }
  }

  async #processBatch(
    batch: RenderClaimBatch,
    signal: AbortSignal,
  ): Promise<void> {
    const rendered: Array<{
      claim: RenderClaim;
      page: RenderedPdfPageDto;
    }> = [];
    let alreadyEncodedBytes = 0;
    try {
      for (const claim of batch.claims) {
        if (signal.aborted) return;
        let sourceBytes: Uint8Array | undefined;
        let source: ArrayBuffer | undefined;
        try {
          const read = this.api.readClaimedSource(
            claim.pageId,
            claim.attemptId,
          );
          try {
            sourceBytes = await withTimeout(read, SOURCE_TIMEOUT_MS, signal);
          } catch (error) {
            void read.then(
              (bytes) => bytes.fill(0),
              () => {},
            );
            throw error;
          }
          source = toOwnedArrayBuffer(sourceBytes);
          sourceBytes.fill(0);
          sourceBytes = undefined;
          const render = this.renderPage(source, claim.pageNumber, {
            maxDimension: claim.limits.maxDimension,
            maxDecodedPixels: claim.limits.maxDecodedPixels,
            maxEncodedBytes: claim.limits.maxEncodedBytes,
            maxTotalEncodedBytes: claim.limits.maxTotalEncodedBytes,
            alreadyEncodedBytes,
          });
          let page: RenderedPdfPageDto;
          try {
            page = await withTimeout(render, RENDER_TIMEOUT_MS, signal);
          } catch (error) {
            void render.then(releaseRenderedPdfPage, () => {});
            throw error;
          }
          alreadyEncodedBytes += page.encodedByteLength;
          rendered.push({ claim, page });
        } catch (error) {
          if (signal.aborted) return;
          if (isLocalRenderFailure(error)) {
            const event = await this.api.reportRenderFailure(
              claim.pageId,
              claim.attemptId,
            );
            this.#emit(event);
          }
        } finally {
          sourceBytes?.fill(0);
          if (source && source.byteLength > 0) new Uint8Array(source).fill(0);
          source = undefined;
        }
      }

      if (signal.aborted || rendered.length === 0) return;
      const events = await withTimeout(
        this.api.submitRenderedBatch(
          rendered.map(({ claim, page }) => toSubmission(claim, page)),
        ),
        SUBMIT_TIMEOUT_MS,
        signal,
      );
      for (const event of events) this.#emit(event);
    } finally {
      for (const { page } of rendered) releaseRenderedPdfPage(page);
    }
  }

  #emit(event: IndexingEvent): void {
    for (const listener of this.#listeners) listener(event);
  }
}

function assertSafeBatch(batch: RenderClaimBatch): void {
  if (batch.claims.length > 2) throw new TypeError('unsafe render batch');
  const first = batch.claims[0];
  if (!first) return;
  const pages = new Set<string>();
  for (const claim of batch.claims) {
    if (
      claim.runId !== first.runId ||
      claim.bookId !== first.bookId ||
      pages.has(claim.pageId)
    ) {
      throw new TypeError('unsafe render batch');
    }
    pages.add(claim.pageId);
  }
}

function isLocalRenderFailure(error: unknown): boolean {
  return (
    error instanceof PdfPageRenderError ||
    !(error instanceof DOMException && error.name === 'AbortError')
  );
}

async function withTimeout<T>(
  operation: Promise<T>,
  timeoutMs: number,
  signal: AbortSignal,
): Promise<T> {
  if (signal.aborted) throw new DOMException('Cancelled', 'AbortError');
  let timeout: ReturnType<typeof setTimeout> | undefined;
  let abort: (() => void) | undefined;
  const interruption = new Promise<never>((_, reject) => {
    timeout = setTimeout(
      () => reject(new DOMException('Timed out', 'TimeoutError')),
      timeoutMs,
    );
    abort = () => reject(new DOMException('Cancelled', 'AbortError'));
    signal.addEventListener('abort', abort, { once: true });
  });
  try {
    return await Promise.race([operation, interruption]);
  } finally {
    if (timeout) clearTimeout(timeout);
    if (abort) signal.removeEventListener('abort', abort);
  }
}

function abortableDelay(milliseconds: number, signal: AbortSignal) {
  return new Promise<void>((resolve) => {
    if (signal.aborted) {
      resolve();
      return;
    }
    const timeout = setTimeout(finish, milliseconds);
    signal.addEventListener('abort', finish, { once: true });
    function finish() {
      clearTimeout(timeout);
      signal.removeEventListener('abort', finish);
      resolve();
    }
  });
}
