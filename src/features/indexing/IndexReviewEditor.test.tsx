import { act, cleanup, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { IndexReviewEditor } from './IndexReviewEditor';

const page = {
  id: '11111111-1111-4111-8111-111111111111',
  runId: '22222222-2222-4222-822222222222',
  bookId: '33333333-3333-4333-833333333333',
  pageNumber: 7,
  qualityReason: 'no_text' as const,
  status: 'needs_review' as const,
  reviewReason: null,
  safeError: null,
  contentVersion: 1,
  blocks: [
    {
      id: '44444444-4444-4444-844444444444',
      ordinal: 0,
      kind: 'transcript' as const,
      source: 'ai_transcribed' as const,
      plainText: 'Synthetic only',
      latex: null,
      tableCells: null,
      visualDescription: null,
      bounds: null,
      contentVersion: 1,
    },
  ],
  corrections: [],
  updatedAt: '2026-08-04T00:00:00.000Z',
};

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

describe('IndexReviewEditor', () => {
  it('renders a local ephemeral preview and saves CAS correction data without provider calls', async () => {
    const user = userEvent.setup();
    const source = new Uint8Array([1, 2, 3]);
    const capture = {
      schemaVersion: 1 as const,
      mimeType: 'image/png' as const,
      width: 1,
      height: 1,
      decodedPixelCount: 1,
      encodedByteLength: 8,
      sha256: 'a'.repeat(64),
      bytes: new Uint8Array([137, 80, 78, 71, 13, 10, 26, 10]),
    };
    const createObjectURL = vi.fn(() => 'blob:synthetic');
    const revokeObjectURL = vi.fn();
    vi.stubGlobal('URL', { createObjectURL, revokeObjectURL });
    const saveCorrection = vi.fn().mockResolvedValue({});
    const changed = vi.fn();
    let renderedSource: ArrayBuffer | undefined;
    const view = render(
      <IndexReviewEditor
        page={page}
        api={{
          saveCorrection,
          resolveCorrectionConflict: vi.fn(),
          deleteCorrection: vi.fn(),
          retryPage: vi.fn(),
        }}
        onChanged={changed}
        readerApi={{ readBookSource: async () => source }}
        renderPage={async (localSource) => {
          renderedSource = localSource;
          const transferred = structuredClone(localSource, {
            transfer: [localSource],
          });
          new Uint8Array(transferred).fill(0);
          return capture;
        }}
      />,
    );

    expect(
      await screen.findByRole('img', { name: 'Local original page 7' }),
    ).toHaveAttribute('src', 'blob:synthetic');
    expect(source.every((byte) => byte === 0)).toBe(true);
    expect(renderedSource?.byteLength).toBe(0);
    expect(capture.bytes.every((byte) => byte === 0)).toBe(true);
    await user.clear(screen.getByLabelText('Editable value'));
    await user.type(
      screen.getByLabelText('Editable value'),
      'Edited synthetic value',
    );
    await user.click(screen.getByRole('button', { name: 'Save correction' }));
    await waitFor(() => expect(saveCorrection).toHaveBeenCalledOnce());
    expect(saveCorrection.mock.calls[0][0]).toMatchObject({
      bookId: page.bookId,
      pageId: page.id,
      targetBlockId: page.blocks[0].id,
      targetContentVersion: 1,
      valueKind: 'text',
      correctedValue: 'Edited synthetic value',
      expectedRevision: 0,
    });
    expect(saveCorrection.mock.calls[0][0].originalValueSha256).toMatch(
      /^[0-9a-f]{64}$/u,
    );
    expect(changed).toHaveBeenCalledOnce();
    expect(() => view.unmount()).not.toThrow();
    expect(revokeObjectURL).toHaveBeenCalledWith('blob:synthetic');
  });

  it('releases a detached late preview after unmount without resurrecting local image state', async () => {
    const source = new Uint8Array([4, 5, 6]);
    const capture = {
      schemaVersion: 1 as const,
      mimeType: 'image/png' as const,
      width: 1,
      height: 1,
      decodedPixelCount: 1,
      encodedByteLength: 8,
      sha256: 'b'.repeat(64),
      bytes: new Uint8Array([137, 80, 78, 71, 13, 10, 26, 10]),
    };
    let resolveRender!: (value: typeof capture) => void;
    let renderedSource: ArrayBuffer | undefined;
    const createObjectURL = vi.fn(() => 'blob:late-preview');
    const revokeObjectURL = vi.fn();
    vi.stubGlobal('URL', { createObjectURL, revokeObjectURL });
    const renderPage = vi.fn((localSource: ArrayBuffer) => {
      renderedSource = localSource;
      const transferred = structuredClone(localSource, {
        transfer: [localSource],
      });
      new Uint8Array(transferred).fill(0);
      return new Promise<typeof capture>((resolve) => {
        resolveRender = resolve;
      });
    });
    const view = render(
      <IndexReviewEditor
        page={page}
        api={{
          saveCorrection: vi.fn(),
          resolveCorrectionConflict: vi.fn(),
          deleteCorrection: vi.fn(),
          retryPage: vi.fn(),
        }}
        onChanged={vi.fn()}
        readerApi={{ readBookSource: async () => source }}
        renderPage={renderPage}
      />,
    );

    await waitFor(() => expect(renderPage).toHaveBeenCalledOnce());
    expect(source.every((byte) => byte === 0)).toBe(true);
    expect(renderedSource?.byteLength).toBe(0);
    expect(() => view.unmount()).not.toThrow();

    await act(async () => {
      resolveRender(capture);
      await Promise.resolve();
    });
    await waitFor(() =>
      expect(capture.bytes.every((byte) => byte === 0)).toBe(true),
    );
    expect(createObjectURL).not.toHaveBeenCalled();
    expect(revokeObjectURL).not.toHaveBeenCalled();
    expect(
      screen.queryByRole('img', { name: 'Local original page 7' }),
    ).not.toBeInTheDocument();
  });

  it('retries once with the safe review version and refreshes only after success', async () => {
    const user = userEvent.setup();
    let resolveRetry!: (attemptId: string) => void;
    const retryPage = vi.fn(
      () =>
        new Promise<string>((resolve) => {
          resolveRetry = resolve;
        }),
    );
    const changed = vi.fn();
    const createObjectURL = vi.fn(() => 'blob:synthetic-retry');
    vi.stubGlobal('URL', { createObjectURL, revokeObjectURL: vi.fn() });
    render(
      <IndexReviewEditor
        page={page}
        api={{
          saveCorrection: vi.fn(),
          resolveCorrectionConflict: vi.fn(),
          deleteCorrection: vi.fn(),
          retryPage,
        }}
        onChanged={changed}
        readerApi={{ readBookSource: async () => new Uint8Array([1]) }}
        renderPage={async () => ({
          schemaVersion: 1,
          mimeType: 'image/png',
          width: 1,
          height: 1,
          decodedPixelCount: 1,
          encodedByteLength: 1,
          sha256: 'a'.repeat(64),
          bytes: new Uint8Array([1]),
        })}
      />,
    );
    await screen.findByRole('img', { name: 'Local original page 7' });

    const retryButton = screen.getByRole('button', { name: 'Retry page' });
    await user.click(retryButton);
    expect(retryButton).toBeDisabled();
    await user.click(retryButton);
    expect(retryPage).toHaveBeenCalledOnce();
    expect(retryPage).toHaveBeenCalledWith(page.id, page.updatedAt);
    expect(changed).not.toHaveBeenCalled();

    resolveRetry('55555555-5555-4555-8555-555555555555');
    await waitFor(() => expect(changed).toHaveBeenCalledOnce());
    expect(retryButton).toBeEnabled();
  });

  it('keeps a stale retry conflict safe and does not refresh', async () => {
    const user = userEvent.setup();
    const changed = vi.fn();
    vi.stubGlobal('URL', {
      createObjectURL: vi.fn(() => 'blob:synthetic-conflict'),
      revokeObjectURL: vi.fn(),
    });
    render(
      <IndexReviewEditor
        page={page}
        api={{
          saveCorrection: vi.fn(),
          resolveCorrectionConflict: vi.fn(),
          deleteCorrection: vi.fn(),
          retryPage: vi
            .fn()
            .mockRejectedValue(new Error('private-attempt-sentinel')),
        }}
        onChanged={changed}
        readerApi={{ readBookSource: async () => new Uint8Array([1]) }}
        renderPage={async () => ({
          schemaVersion: 1,
          mimeType: 'image/png',
          width: 1,
          height: 1,
          decodedPixelCount: 1,
          encodedByteLength: 1,
          sha256: 'a'.repeat(64),
          bytes: new Uint8Array([1]),
        })}
      />,
    );
    await screen.findByRole('img', { name: 'Local original page 7' });

    await user.click(screen.getByRole('button', { name: 'Retry page' }));
    expect(
      await screen.findByText(
        'The page could not be retried. Review the durable run state and try again.',
      ),
    ).toHaveAttribute('role', 'alert');
    expect(
      screen.queryByText(/private-attempt-sentinel/u),
    ).not.toBeInTheDocument();
    expect(changed).not.toHaveBeenCalled();
  });
});
