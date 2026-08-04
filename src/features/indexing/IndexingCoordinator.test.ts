import { afterEach, describe, expect, it, vi } from 'vitest';

import { clearMocks, installTauriMock } from '../../test/tauri-mock';
import type { RenderedPdfPageDto } from './indexing-contract';
import {
  IndexingCoordinator,
  type LocalPageRenderer,
} from './IndexingCoordinator';
import {
  TauriIndexingApi,
  type IndexingApi,
  type IndexingEvent,
  type RenderClaim,
  type RenderClaimBatch,
  type RenderedPageSubmission,
} from './api';

const RUN_ID = '11111111-1111-4111-8111-111111111111';
const BOOK_ID = '22222222-2222-4222-8222-222222222222';

afterEach(() => {
  vi.useRealTimers();
  clearMocks();
});

function claim(index: number): RenderClaim {
  return {
    runId: RUN_ID,
    bookId: BOOK_ID,
    pageId: `33333333-3333-4333-8333-33333333333${index}`,
    pageNumber: index + 1,
    attemptId: `44444444-4444-4444-8444-44444444444${index}`,
    limits: {
      maxDimension: 4096,
      maxDecodedPixels: 8_847_360,
      maxEncodedBytes: 4 * 1024 * 1024,
      maxTotalEncodedBytes: 12 * 1024 * 1024,
    },
  };
}

function rendered(unique: number): RenderedPdfPageDto {
  const bytes = new Uint8Array([137, 80, 78, 71, unique]);
  return {
    schemaVersion: 1,
    mimeType: 'image/png',
    width: 1,
    height: 1,
    decodedPixelCount: 1,
    encodedByteLength: bytes.byteLength,
    sha256: `${unique}`.padStart(64, '0'),
    bytes,
  };
}

class FakeApi implements IndexingApi {
  batches: RenderClaimBatch[] = [];
  submissions: RenderedPageSubmission[][] = [];
  sources: Uint8Array[] = [];
  renderFailures: string[] = [];
  calls: string[] = [];

  async confirmOperation(): Promise<string> {
    throw new Error('unused');
  }
  async createRun(): Promise<string> {
    throw new Error('unused');
  }
  async authorizeRun(): Promise<void> {
    throw new Error('unused');
  }
  async claimRenderBatch(): Promise<RenderClaimBatch> {
    this.calls.push('claim');
    return this.batches.shift() ?? { claims: [] };
  }
  async readClaimedSource(pageId: string): Promise<Uint8Array> {
    this.calls.push(`source:${pageId}`);
    const source = new Uint8Array([1, 2, 3]);
    this.sources.push(source);
    return source;
  }
  async submitRenderedBatch(
    submissions: RenderedPageSubmission[],
  ): Promise<IndexingEvent[]> {
    this.calls.push('submit');
    this.submissions.push(submissions);
    return submissions.map((submission) => ({
      runId: submission.runId,
      pageId: submission.pageId,
      status: 'parsing',
      safeErrorCode: null,
    }));
  }
  async reportRenderFailure(pageId: string): Promise<IndexingEvent> {
    this.calls.push(`failure:${pageId}`);
    this.renderFailures.push(pageId);
    return {
      runId: RUN_ID,
      pageId,
      status: 'failed',
      safeErrorCode: 'INDEX_RENDER_FAILED',
    };
  }
  async pauseRun(): Promise<void> {}
  async resumeRun(): Promise<void> {}
  async cancelRun(): Promise<void> {
    this.calls.push('cancel');
  }
  async retryPage(): Promise<string> {
    throw new Error('unused');
  }
}

describe('IndexingCoordinator', () => {
  it('uses a wiped raw IPC body for rendered bytes', async () => {
    let transmitted: Uint8Array | undefined;
    const calls = installTauriMock((_command, payload) => {
      if (payload instanceof Uint8Array) transmitted = Uint8Array.from(payload);
      return [
        {
          runId: RUN_ID,
          pageId: claim(0).pageId,
          status: 'parsing',
          safeErrorCode: null,
        },
      ];
    });
    const page = rendered(9);
    const submission: RenderedPageSubmission = {
      ...claim(0),
      schemaVersion: 1,
      mimeType: 'image/png',
      width: page.width,
      height: page.height,
      decodedPixelCount: page.decodedPixelCount,
      encodedByteLength: page.encodedByteLength,
      sha256: page.sha256,
      bytes: page.bytes,
    };
    const events = await new TauriIndexingApi().submitRenderedBatch([
      submission,
    ]);

    expect(events[0].status).toBe('parsing');
    expect(Array.from(transmitted ?? [])).toEqual([137, 80, 78, 71, 9]);
    expect(calls[0].payload).toBeInstanceOf(Uint8Array);
    expect(
      Array.from(calls[0].payload as Uint8Array).every((byte) => byte === 0),
    ).toBe(true);
    expect(Array.from(page.bytes)).toEqual([137, 80, 78, 71, 9]);
  });

  it('wipes the raw IPC body when submission fails', async () => {
    const calls = installTauriMock(() => {
      throw new Error('synthetic IPC failure');
    });
    const page = rendered(8);
    const submission: RenderedPageSubmission = {
      ...claim(0),
      schemaVersion: 1,
      mimeType: 'image/png',
      width: page.width,
      height: page.height,
      decodedPixelCount: page.decodedPixelCount,
      encodedByteLength: page.encodedByteLength,
      sha256: page.sha256,
      bytes: page.bytes,
    };

    await expect(
      new TauriIndexingApi().submitRenderedBatch([submission]),
    ).rejects.toBeDefined();
    expect(
      Array.from(calls[0].payload as Uint8Array).every((byte) => byte === 0),
    ).toBe(true);
  });

  it('renders a same-run bounded batch, submits once, and wipes all bytes', async () => {
    vi.useFakeTimers();
    const api = new FakeApi();
    api.batches.push({ claims: [claim(0), claim(1)] });
    const pages = [rendered(1), rendered(2)];
    const renderer = vi.fn<LocalPageRenderer>(async () => pages.shift()!);
    const coordinator = new IndexingCoordinator(api, renderer);
    const events: IndexingEvent[] = [];
    coordinator.subscribe((event) => events.push(event));

    coordinator.start();
    await vi.waitFor(() => expect(api.submissions).toHaveLength(1));
    expect(api.submissions[0]).toHaveLength(2);
    expect(renderer).toHaveBeenCalledTimes(2);
    expect(
      api.sources.every((source) => source.every((byte) => byte === 0)),
    ).toBe(true);
    expect(
      renderer.mock.calls.every(([source]) =>
        new Uint8Array(source).every((byte) => byte === 0),
      ),
    ).toBe(true);
    expect(events.map((event) => event.status)).toEqual(['parsing', 'parsing']);
    const submittedBytes = api.submissions[0].map((item) => item.bytes);
    await vi.waitFor(() =>
      expect(
        submittedBytes.every((bytes) => bytes.every((byte) => byte === 0)),
      ).toBe(true),
    );
    await coordinator.stop();
    expect(api.calls).not.toContain('cancel');
  });

  it('releases a render that resolves after global shutdown without cancelling Rust', async () => {
    const api = new FakeApi();
    api.batches.push({ claims: [claim(0)] });
    let resolveRender: ((page: RenderedPdfPageDto) => void) | undefined;
    const lateRender = new Promise<RenderedPdfPageDto>((resolve) => {
      resolveRender = resolve;
    });
    const renderer = vi.fn<LocalPageRenderer>(() => lateRender);
    const coordinator = new IndexingCoordinator(api, renderer);
    const page = rendered(7);

    coordinator.start();
    await vi.waitFor(() => expect(renderer).toHaveBeenCalledOnce());
    await coordinator.stop();
    resolveRender?.(page);
    await Promise.resolve();
    await Promise.resolve();

    expect(page.bytes.every((byte) => byte === 0)).toBe(true);
    expect(api.submissions).toHaveLength(0);
    expect(api.calls).not.toContain('cancel');
  });

  it('reports one local render failure and still submits the unrelated page', async () => {
    vi.useFakeTimers();
    const api = new FakeApi();
    api.batches.push({ claims: [claim(0), claim(1)] });
    const renderer = vi
      .fn<LocalPageRenderer>()
      .mockRejectedValueOnce(new Error('synthetic render failure'))
      .mockResolvedValueOnce(rendered(2));
    const coordinator = new IndexingCoordinator(api, renderer);

    coordinator.start();
    await vi.waitFor(() => expect(api.submissions).toHaveLength(1));
    expect(api.renderFailures).toEqual([claim(0).pageId]);
    expect(api.submissions[0].map((item) => item.pageId)).toEqual([
      claim(1).pageId,
    ]);
    await coordinator.stop();
  });

  it('rejects cross-run batches before reading source or rendering', async () => {
    vi.useFakeTimers();
    const api = new FakeApi();
    const substituted = {
      ...claim(1),
      runId: '55555555-5555-4555-8555-555555555555',
    };
    api.batches.push({ claims: [claim(0), substituted] });
    const renderer = vi.fn<LocalPageRenderer>();
    const coordinator = new IndexingCoordinator(api, renderer);

    coordinator.start();
    await vi.advanceTimersByTimeAsync(1_100);
    expect(renderer).not.toHaveBeenCalled();
    expect(api.calls.some((call) => call.startsWith('source:'))).toBe(false);
    await coordinator.stop();
  });
});
