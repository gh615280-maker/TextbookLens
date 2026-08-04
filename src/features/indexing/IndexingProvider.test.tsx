import { StrictMode } from 'react';
import { act, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { IndexingCoordinator } from './IndexingCoordinator';
import { IndexingProvider, useIndexingCoordinator } from './IndexingProvider';
import type { IndexingApi } from './api';

function Child({ label }: { label: string }) {
  useIndexingCoordinator();
  return <span>{label}</span>;
}

function idleApi(): IndexingApi {
  return {
    confirmOperation: async () => {
      throw new Error('unused');
    },
    createRun: async () => {
      throw new Error('unused');
    },
    authorizeRun: async () => {},
    claimRenderBatch: async () => ({ claims: [] }),
    readClaimedSource: async () => new Uint8Array(),
    submitRenderedBatch: async () => [],
    reportRenderFailure: async () => {
      throw new Error('unused');
    },
    pauseRun: async () => {},
    resumeRun: async () => {},
    cancelRun: async () => {},
    retryPage: async () => {
      throw new Error('unused');
    },
    getRunAggregate: async () => {
      throw new Error('unused');
    },
    listPageReviews: async () => {
      throw new Error('unused');
    },
    getPageReview: async () => {
      throw new Error('unused');
    },
    listPageCorrections: async () => {
      throw new Error('unused');
    },
    saveCorrection: async () => {
      throw new Error('unused');
    },
    resolveCorrectionConflict: async () => {
      throw new Error('unused');
    },
    deleteCorrection: async () => {
      throw new Error('unused');
    },
  };
}

describe('IndexingProvider', () => {
  it('keeps one global coordinator alive across child route replacement', async () => {
    vi.useFakeTimers();
    const coordinator = new IndexingCoordinator(idleApi(), vi.fn());
    const stop = vi.spyOn(coordinator, 'stop');
    const view = render(
      <IndexingProvider coordinator={coordinator}>
        <Child label="first" />
      </IndexingProvider>,
    );
    expect(screen.getByText('first')).toBeInTheDocument();
    expect(coordinator.running).toBe(true);

    view.rerender(
      <IndexingProvider coordinator={coordinator}>
        <Child label="second" />
      </IndexingProvider>,
    );
    expect(screen.getByText('second')).toBeInTheDocument();
    expect(coordinator.running).toBe(true);
    expect(stop).not.toHaveBeenCalled();

    view.unmount();
    await act(async () => {
      await Promise.resolve();
    });
    expect(stop).toHaveBeenCalledOnce();
    vi.useRealTimers();
  });

  it('remains running after React strict-effect replay', async () => {
    vi.useFakeTimers();
    const coordinator = new IndexingCoordinator(idleApi(), vi.fn());
    const view = render(
      <StrictMode>
        <IndexingProvider coordinator={coordinator}>
          <Child label="strict" />
        </IndexingProvider>
      </StrictMode>,
    );
    await act(async () => {
      await Promise.resolve();
    });
    expect(coordinator.running).toBe(true);
    view.unmount();
    await act(async () => {
      await Promise.resolve();
    });
    expect(coordinator.running).toBe(false);
    vi.useRealTimers();
  });
});
