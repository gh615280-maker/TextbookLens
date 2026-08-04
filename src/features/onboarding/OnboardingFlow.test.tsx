import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { MemoryRouter, Route, Routes } from 'react-router-dom';
import { afterEach, describe, expect, it } from 'vitest';

import { LanguageProvider } from '../../app/LanguageProvider';
import type { BookSummary } from '../../lib/generated/book';
import type { OnboardingStateDto } from '../../lib/generated/onboarding';
import type {
  ProviderCapabilityRegistryDto,
  ProviderProfileSummary,
} from '../../lib/generated/provider';
import type { AppSettingsDto } from '../../lib/generated/settings';
import { clearMocks, installTauriMock } from '../../test/tauri-mock';
import type { ImportCoordinatorPort } from '../import/ImportCoordinator';
import type { ProviderApi, SaveProviderProfileRequest } from '../providers/api';
import { OnboardingPage } from './OnboardingPage';
import type { OnboardingApi } from './onboarding-api';
import { initialOnboardingViewState } from './onboarding-state';

const BOOK_ID = '4f9a2c86-0da8-4dd4-a255-39b4cff89c66';
const PROFILE_ID = '7aa78e91-d74d-4742-92f0-6dc41a1258f7';
const INVALID_KEY = 'synthetic-invalid-onboarding-key';
const VALID_KEY = 'synthetic-valid-onboarding-key';

const parsingBook: BookSummary = {
  id: BOOK_ID,
  title: 'Synthetic lifecycle textbook',
  originalFilename: 'synthetic.docx',
  author: null,
  language: 'en',
  format: 'docx',
  importStatus: 'parsing',
  importErrorCode: null,
  importErrorMessage: null,
  importErrorStage: null,
  readingProgress: 0,
  indexAggregate: {
    status: 'not_required',
    totalPages: 0,
    indexedPages: 0,
    reviewPages: 0,
    failedPages: 0,
  },
  createdAt: '2026-08-03T00:00:00Z',
  updatedAt: '2026-08-03T00:00:00Z',
  lastOpenedAt: null,
};
const readyBook: BookSummary = { ...parsingBook, importStatus: 'ready' };
const profile: ProviderProfileSummary = {
  id: PROFILE_ID,
  kind: 'openai',
  displayName: 'Synthetic OpenAI',
  modelId: 'gpt-5.6',
  contextWindowTokens: 32_000,
  isActive: true,
  credentialStatus: 'available',
  validatedAt: '2026-08-03T00:00:00Z',
};
const registry: ProviderCapabilityRegistryDto = {
  schemaVersion: 1,
  providers: [
    {
      kind: 'openai',
      displayName: 'OpenAI',
      defaultModel: 'gpt-5.6',
      models: [
        {
          id: 'gpt-5.6',
          displayName: 'GPT-5.6',
          contextWindowTokens: 1_050_000,
          defaultMaxOutputTokens: 128_000,
          textChat: 'supported',
          imageInput: 'supported',
          pdfInput: 'supported',
          strictStructuredOutput: 'supported',
          imageLimits: null,
          lastVerified: '2026-08-03',
        },
      ],
    },
  ],
};
const settings: AppSettingsDto = {
  onboardingCompleted: false,
  activeProviderProfileId: null,
  defaultLearningProfileId: null,
  defaultVisionProfileId: null,
  theme: 'system',
  contextMode: 'standard',
  uiLanguage: 'en',
  uiLanguageInitialized: true,
  firstReaderHintCompleted: false,
};

function state(
  selectedBook: BookSummary | null,
  learningProfileConnected: boolean,
): OnboardingStateDto {
  return {
    step: selectedBook
      ? learningProfileConnected
        ? 'ready'
        : 'provider'
      : 'book',
    selectedBook,
    hasReadyBook: selectedBook?.importStatus === 'ready',
    learningProfileConnected,
    visionProfileConnected: false,
    localTextQuality: selectedBook ? 'pending' : 'unavailable',
    canSkipOnboarding:
      selectedBook?.importStatus === 'ready' && learningProfileConnected,
  };
}

function renderFlow(
  onboardingApi: OnboardingApi,
  providerApi: ProviderApi,
  coordinator: ImportCoordinatorPort,
) {
  const languageApi = {
    getAppSettings: async () => settings,
    initializeUiLanguage: async () => settings,
    updateUiLanguage: async () => settings,
  };
  return render(
    <LanguageProvider api={languageApi} detectedLanguages={['en']}>
      <MemoryRouter initialEntries={['/onboarding']}>
        <Routes>
          <Route
            path="/onboarding"
            element={
              <OnboardingPage
                api={onboardingApi}
                providerApi={providerApi}
                importCoordinator={coordinator}
              />
            }
          />
          <Route
            path="/books/:bookId/read"
            element={<h1>Reader destination</h1>}
          />
        </Routes>
      </MemoryRouter>
    </LanguageProvider>,
  );
}

function unusedCoordinator(): ImportCoordinatorPort {
  return {
    importDocument: async () => readyBook,
    retryDocument: async () => readyBook,
    cancel: async () => {},
    cancelPending: () => {},
  };
}

function providerApi(
  validateAndSave: (
    request: SaveProviderProfileRequest,
  ) => Promise<ProviderProfileSummary> = async () => profile,
): ProviderApi {
  return {
    listCapabilities: async () => registry,
    listProfiles: async () => [],
    getSettings: async () => settings,
    validateAndSave,
    replaceCredential: async () => profile,
    deleteProfile: async () => {},
    setDefault: async () => {},
    updateConsent: async () => {},
    resetConsents: async () => {},
  };
}

afterEach(() => {
  cleanup();
  clearMocks();
});

describe('onboarding lifecycle flow', () => {
  it('keeps the book while import and one deduplicated validation finish, then reads', async () => {
    installTauriMock((command) => {
      if (command === 'plugin:dialog|open') return 'C:/synthetic.docx';
      throw new Error(`Unexpected command: ${command}`);
    });
    let releaseImport!: () => void;
    const importGate = new Promise<void>((resolve) => {
      releaseImport = resolve;
    });
    const coordinator: ImportCoordinatorPort = {
      async importDocument(_source, _progress, identified) {
        identified(parsingBook);
        await importGate;
        return readyBook;
      },
      retryDocument: async () => readyBook,
      cancel: async () => {},
      cancelPending: () => {},
    };
    let connected = false;
    const onboardingApi: OnboardingApi = {
      getState: async () =>
        connected ? state(parsingBook, true) : state(null, false),
    };
    let releaseInvalid!: () => void;
    const invalidGate = new Promise<void>((resolve) => {
      releaseInvalid = resolve;
    });
    let validationCount = 0;
    const api = providerApi(async (request) => {
      validationCount += 1;
      const invalid = request.credential === INVALID_KEY;
      const valid = request.credential === VALID_KEY;
      request.credential = '';
      if (invalid) {
        await invalidGate;
        throw {
          code: 'INVALID_API_KEY',
          message: 'Invalid key',
          nextStep: 'Check the synthetic key',
          diagnosticId: null,
        };
      }
      expect(valid).toBe(true);
      connected = true;
      return profile;
    });
    const user = userEvent.setup();
    renderFlow(onboardingApi, api, coordinator);

    await user.click(await screen.findByRole('button', { name: '导入教材' }));
    expect(
      await screen.findByText(/Book: Synthetic lifecycle textbook/),
    ).toBeVisible();
    expect(screen.getByText(/local import continues/i)).toBeVisible();

    const keyInput = screen.getByLabelText('Key');
    await user.type(keyInput, INVALID_KEY);
    const form = screen.getByRole('form', { name: 'Connect AI provider' });
    fireEvent.submit(form);
    fireEvent.submit(form);
    expect(validationCount).toBe(1);
    releaseInvalid();
    expect(await screen.findByRole('alert')).toHaveTextContent('Invalid key');
    expect(keyInput).toHaveValue(INVALID_KEY);
    expect(screen.getByText(/Synthetic lifecycle textbook/)).toBeVisible();

    await user.clear(keyInput);
    await user.type(keyInput, VALID_KEY);
    await user.click(
      screen.getByRole('button', { name: 'Validate & Connect' }),
    );
    expect(await screen.findByText('Still importing locally')).toBeVisible();
    expect(
      screen.getByRole('button', { name: 'Start reading' }),
    ).toBeDisabled();
    expect(validationCount).toBe(2);

    releaseImport();
    const startReading = screen.getByRole('button', { name: 'Start reading' });
    await waitFor(() => expect(startReading).toBeEnabled());
    expect(document.body.innerHTML).not.toContain(INVALID_KEY);
    expect(document.body.innerHTML).not.toContain(VALID_KEY);
    expect(JSON.stringify(initialOnboardingViewState)).not.toContain(
      'credential',
    );
    await user.click(startReading);
    expect(
      await screen.findByRole('heading', { name: 'Reader destination' }),
    ).toBeVisible();
  });

  it('ignores a late validation after route unmount and releases mounted secret UI', async () => {
    let stateReads = 0;
    const onboardingApi: OnboardingApi = {
      getState: async () => {
        stateReads += 1;
        return state(parsingBook, false);
      },
    };
    let finishValidation!: () => void;
    const validationGate = new Promise<void>((resolve) => {
      finishValidation = resolve;
    });
    let validationCount = 0;
    const api = providerApi(async (request) => {
      validationCount += 1;
      expect(request.credential).toBe(VALID_KEY);
      request.credential = '';
      await validationGate;
      return profile;
    });
    const user = userEvent.setup();
    const view = renderFlow(onboardingApi, api, unusedCoordinator());
    const input = await screen.findByLabelText('Key');
    await user.type(input, VALID_KEY);
    fireEvent.submit(screen.getByRole('form', { name: 'Connect AI provider' }));
    expect(validationCount).toBe(1);

    view.unmount();
    expect(document.body.innerHTML).not.toContain(VALID_KEY);
    await act(async () => {
      finishValidation();
      await validationGate;
      await Promise.resolve();
    });
    expect(stateReads).toBe(1);
  });

  it('does not skip setup for ready profile metadata when the credential is missing', async () => {
    const onboardingApi: OnboardingApi = {
      getState: async () => state(readyBook, false),
    };
    renderFlow(onboardingApi, providerApi(), unusedCoordinator());

    expect(
      await screen.findByRole('heading', { name: 'Connect an AI service' }),
    ).toBeVisible();
    expect(
      screen.queryByRole('button', { name: 'Start reading' }),
    ).not.toBeInTheDocument();
  });
});
