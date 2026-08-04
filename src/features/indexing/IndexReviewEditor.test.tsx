import { render, screen, waitFor } from '@testing-library/react';
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

afterEach(() => vi.unstubAllGlobals());

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
        renderPage={async () => capture}
      />,
    );

    expect(
      await screen.findByRole('img', { name: 'Local original page 7' }),
    ).toHaveAttribute('src', 'blob:synthetic');
    expect(source.every((byte) => byte === 0)).toBe(true);
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
    view.unmount();
    expect(revokeObjectURL).toHaveBeenCalledWith('blob:synthetic');
  });
});
