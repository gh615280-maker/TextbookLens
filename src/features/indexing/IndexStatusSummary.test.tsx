import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { IndexPageList } from './IndexPageList';
import { IndexStatusSummary } from './IndexStatusSummary';

const RUN_ID = '11111111-1111-4111-8111-111111111111';
const BOOK_ID = '22222222-2222-4222-822222222222';

describe('Index quality status', () => {
  it('shows SQLite aggregate truth and completed, review, and failed counts', () => {
    render(
      <IndexStatusSummary
        aggregate={{
          runId: RUN_ID,
          bookId: BOOK_ID,
          controlStatus: 'paused',
          aggregateStatus: 'needs_review',
          pages: {
            total: 5,
            notRequired: 1,
            queued: 0,
            rendering: 0,
            sending: 0,
            parsing: 0,
            validating: 0,
            indexed: 2,
            needsReview: 1,
            failed: 1,
            cancelled: 0,
          },
          updatedAt: '2026-08-04T00:00:00.000Z',
        }}
      />,
    );
    expect(screen.getByRole('status')).toHaveTextContent(
      'Overall: needs review. Control: paused.',
    );
    expect(screen.getByText('Completed').parentElement).toHaveTextContent('3');
    expect(screen.getByText('Needs review').parentElement).toHaveTextContent(
      '1',
    );
    expect(screen.getByText('Failed').parentElement).toHaveTextContent('1');
  });

  it('does not list completed or in-flight pages', () => {
    const page = (
      id: string,
      pageNumber: number,
      status: 'indexed' | 'needs_review' | 'failed',
    ) => ({
      id,
      runId: RUN_ID,
      bookId: BOOK_ID,
      pageNumber,
      qualityReason: 'no_text' as const,
      status,
      reviewReason: null,
      safeError: null,
      contentVersion: 1,
      blocks: [],
      corrections: [],
      updatedAt: '2026-08-04T00:00:00.000Z',
    });
    render(
      <IndexPageList
        pages={[
          page('33333333-3333-4333-833333333333', 1, 'indexed'),
          page('44444444-4444-4444-844444444444', 2, 'needs_review'),
          page('55555555-5555-4555-855555555555', 3, 'failed'),
        ]}
        selectedPageId={null}
        onSelect={() => {}}
      />,
    );
    expect(screen.queryByRole('button', { name: /Page 1/ })).toBeNull();
    expect(screen.getByRole('button', { name: /Page 2/ })).toBeVisible();
    expect(screen.getByRole('button', { name: /Page 3/ })).toBeVisible();
  });
});
