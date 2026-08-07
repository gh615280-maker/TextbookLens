import { StrictMode, type ReactNode } from 'react';
import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { LanguageContext } from '../../app/LanguageProvider';
import { formatMessage } from '../../lib/i18n';
import { IndexStartConfirmation } from './IndexStartConfirmation';

const RUN_ID = '44444444-4444-4444-8444-444444444444';
const request = {
  runId: null,
  bookId: '11111111-1111-4111-8111-111111111111',
  sourceSha256: 'a'.repeat(64),
  providerProfileId: '22222222-2222-4222-8222-222222222222',
  pages: [
    { pageNumber: 3, qualityReason: 'no_text' as const, localTextSha256: null },
  ],
};

afterEach(cleanup);

describe('IndexStartConfirmation', () => {
  it('uses one fresh confirmation token for one explicit start and keeps the preference profile-scoped', async () => {
    const user = userEvent.setup();
    const confirmOperation = vi
      .fn()
      .mockResolvedValue('33333333-3333-4333-8333-333333333333');
    const createRun = vi
      .fn()
      .mockResolvedValue('44444444-4444-4444-8444-444444444444');
    const authorizeRun = vi.fn();
    const setNoPrompt = vi.fn().mockResolvedValue(undefined);
    const started = vi.fn();
    renderConfirmation(
      <IndexStartConfirmation
        request={request}
        profileName="Vision profile"
        modelName="Model X"
        api={{ confirmOperation, createRun, authorizeRun }}
        onReject={vi.fn()}
        onSetProfileNoPrompt={setNoPrompt}
        onStarted={started}
      />,
    );

    expect(
      screen.getByText(/Profile: Vision profile\. Model: Model X\. Pages: 1/),
    ).toBeVisible();
    expect(screen.getByText(/Local page images/)).toBeVisible();
    await user.click(screen.getByRole('checkbox'));
    await user.click(screen.getByRole('button', { name: 'Confirm and start' }));

    expect(confirmOperation).toHaveBeenCalledTimes(1);
    expect(confirmOperation).toHaveBeenCalledWith(request);
    expect(setNoPrompt).toHaveBeenCalledWith(request.providerProfileId);
    expect(createRun).toHaveBeenCalledWith(
      '33333333-3333-4333-8333-333333333333',
      request,
    );
    expect(authorizeRun).not.toHaveBeenCalled();
    expect(started).toHaveBeenCalledWith(
      '44444444-4444-4444-8444-444444444444',
    );
  });

  it('rejects without confirming, creating a run, or changing consent', async () => {
    const user = userEvent.setup();
    const confirmOperation = vi.fn();
    const createRun = vi.fn();
    const authorizeRun = vi.fn();
    const setNoPrompt = vi.fn();
    const reject = vi.fn();
    renderConfirmation(
      <IndexStartConfirmation
        request={request}
        profileName="Vision profile"
        modelName="Model X"
        api={{ confirmOperation, createRun, authorizeRun }}
        onReject={reject}
        onSetProfileNoPrompt={setNoPrompt}
        onStarted={vi.fn()}
      />,
    );

    await user.click(
      screen.getByRole('button', { name: 'Continue without AI indexing' }),
    );

    expect(reject).toHaveBeenCalledOnce();
    expect(confirmOperation).not.toHaveBeenCalled();
    expect(createRun).not.toHaveBeenCalled();
    expect(authorizeRun).not.toHaveBeenCalled();
    expect(setNoPrompt).not.toHaveBeenCalled();
  });

  it('guards duplicate submits before creating exactly one run', async () => {
    let releaseToken: (token: string) => void = () => {};
    const confirmOperation = vi.fn(
      () =>
        new Promise<string>((resolve) => {
          releaseToken = resolve;
        }),
    );
    const createRun = vi.fn().mockResolvedValue(RUN_ID);
    const authorizeRun = vi.fn();
    const started = vi.fn();
    renderConfirmation(
      <IndexStartConfirmation
        request={request}
        profileName="Vision profile"
        modelName="Model X"
        api={{ confirmOperation, createRun, authorizeRun }}
        onReject={vi.fn()}
        onStarted={started}
      />,
    );
    const start = screen.getByRole('button', { name: 'Confirm and start' });

    fireEvent.click(start);
    fireEvent.click(start);
    expect(confirmOperation).toHaveBeenCalledOnce();

    releaseToken('33333333-3333-4333-8333-333333333333');
    await waitFor(() => expect(createRun).toHaveBeenCalledOnce());
    expect(started).toHaveBeenCalledWith(RUN_ID);
  });

  it('starts a run after the StrictMode effect cleanup and remount check', async () => {
    const confirmOperation = vi
      .fn()
      .mockResolvedValue('33333333-3333-4333-8333-333333333333');
    const createRun = vi.fn().mockResolvedValue(RUN_ID);
    const authorizeRun = vi.fn();
    const started = vi.fn();

    renderConfirmation(
      <StrictMode>
        <IndexStartConfirmation
          request={request}
          profileName="Vision profile"
          modelName="Model X"
          api={{ confirmOperation, createRun, authorizeRun }}
          onReject={vi.fn()}
          onStarted={started}
        />
      </StrictMode>,
    );

    fireEvent.click(screen.getByRole('button', { name: 'Confirm and start' }));

    await waitFor(() => expect(createRun).toHaveBeenCalledOnce());
    expect(started).toHaveBeenCalledWith(RUN_ID);
  });

  it('does not continue a late confirmation after unmount', async () => {
    let releaseToken: (token: string) => void = () => {};
    const confirmOperation = vi.fn(
      () =>
        new Promise<string>((resolve) => {
          releaseToken = resolve;
        }),
    );
    const createRun = vi.fn();
    const authorizeRun = vi.fn();
    const view = renderConfirmation(
      <IndexStartConfirmation
        request={request}
        profileName="Vision profile"
        modelName="Model X"
        api={{ confirmOperation, createRun, authorizeRun }}
        onReject={vi.fn()}
        onStarted={vi.fn()}
      />,
    );

    fireEvent.click(screen.getByRole('button', { name: 'Confirm and start' }));
    view.unmount();
    releaseToken('33333333-3333-4333-8333-333333333333');
    await Promise.resolve();
    await Promise.resolve();

    expect(createRun).not.toHaveBeenCalled();
    expect(authorizeRun).not.toHaveBeenCalled();
  });

  it('uses a fresh token to reauthorize an unfinished durable run', async () => {
    const resumedRequest = { ...request, runId: RUN_ID };
    const confirmOperation = vi
      .fn()
      .mockResolvedValue('33333333-3333-4333-8333-333333333333');
    const createRun = vi.fn();
    const authorizeRun = vi.fn().mockResolvedValue(undefined);
    const started = vi.fn();
    renderConfirmation(
      <IndexStartConfirmation
        request={resumedRequest}
        profileName="Vision profile"
        modelName="Model X"
        api={{ confirmOperation, createRun, authorizeRun }}
        onReject={vi.fn()}
        onStarted={started}
      />,
    );

    fireEvent.click(screen.getByRole('button', { name: 'Confirm and start' }));

    await waitFor(() => expect(authorizeRun).toHaveBeenCalledOnce());
    expect(authorizeRun).toHaveBeenCalledWith(
      RUN_ID,
      '33333333-3333-4333-8333-333333333333',
      resumedRequest,
    );
    expect(createRun).not.toHaveBeenCalled();
    expect(started).toHaveBeenCalledWith(RUN_ID);
  });
});

function renderConfirmation(children: ReactNode) {
  return render(
    <LanguageContext.Provider
      value={{
        uiLanguage: 'en',
        isLoading: false,
        statusMessage: null,
        switchLanguage: async () => {},
        message: (key, values) => formatMessage('en', key, values),
      }}
    >
      {children}
    </LanguageContext.Provider>,
  );
}
