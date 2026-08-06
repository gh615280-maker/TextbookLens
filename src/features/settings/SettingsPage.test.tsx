import {
  cleanup,
  render,
  screen,
  waitFor,
  within,
} from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { LanguageProvider } from '../../app/LanguageProvider';
import type { AppSettingsDto } from '../../lib/generated/settings';
import type { SettingsApi } from './api';
import { SettingsPage } from './SettingsPage';

const languageSettings: AppSettingsDto = {
  onboardingCompleted: true,
  activeProviderProfileId: null,
  defaultLearningProfileId: null,
  defaultVisionProfileId: null,
  theme: 'system',
  contextMode: 'standard',
  uiLanguage: 'en',
  uiLanguageInitialized: true,
  firstReaderHintCompleted: true,
};

function api(): SettingsApi {
  return {
    getMaintenanceStatus: vi.fn().mockResolvedValue({
      code: 'MAINTENANCE_AVAILABLE',
      activeOperations: [],
    }),
    getStorageUsage: vi.fn().mockResolvedValue({
      totalBytes: 2048n,
      totalFileCount: 2n,
      categories: [
        { category: 'database', bytes: 1024n, fileCount: 1n },
        { category: 'source', bytes: 1024n, fileCount: 1n },
      ],
    }),
    openAppDataDirectory: vi.fn().mockResolvedValue(undefined),
    createLocalBackup: vi.fn().mockResolvedValue({
      formatVersion: 1,
      archiveBytes: 2048n,
      entryCount: 2n,
    }),
    restoreLocalBackup: vi.fn().mockResolvedValue({
      status: 'RESTORE_READY_TO_RESTART',
      restartRequired: true,
      aiConfigurationRequired: true,
    }),
    clearAllTextbookLensData: vi.fn().mockResolvedValue({
      status: 'CLEAR_READY_TO_RESTART',
      restartRequired: true,
    }),
    getReaderSettings: vi.fn().mockResolvedValue({
      theme: 'system',
      fontScale: 1,
      lineHeight: 1.5,
      readerWidth: 72,
      pdfZoom: 1,
    }),
    updateReaderSettings: vi
      .fn()
      .mockImplementation(async (settings) => settings),
    restartApplication: vi.fn().mockResolvedValue(undefined),
  };
}

function renderSettings(
  settingsApi = api(),
  picker = {
    pickBackupDestination: vi.fn().mockResolvedValue(null),
    pickBackupToRestore: vi.fn().mockResolvedValue(null),
  },
) {
  render(
    <LanguageProvider
      api={{
        getAppSettings: async () => languageSettings,
        initializeUiLanguage: async () => languageSettings,
        updateUiLanguage: async () => languageSettings,
      }}
      detectedLanguages={['en']}
    >
      <SettingsPage api={settingsApi} picker={picker} />
    </LanguageProvider>,
  );
  return { settingsApi, picker };
}

describe('SettingsPage', () => {
  afterEach(cleanup);

  it('shows only safe storage summaries and opens or refreshes local storage', async () => {
    const { settingsApi } = renderSettings();
    expect(
      await screen.findByRole('heading', { name: 'Data and privacy' }),
    ).toBeVisible();
    expect(screen.getByText('2 KB')).toBeVisible();
    expect(screen.queryByText(/C:\\|Users|fixture/i)).not.toBeInTheDocument();
    await userEvent
      .setup()
      .click(screen.getByRole('button', { name: 'Open data directory' }));
    expect(settingsApi.openAppDataDirectory).toHaveBeenCalledOnce();
    await userEvent
      .setup()
      .click(screen.getByRole('button', { name: 'Refresh' }));
    await waitFor(() => expect(settingsApi.getStorageUsage).toHaveBeenCalled());
  });

  it('does not call backup for a cancelled save dialog and prevents duplicate creation', async () => {
    const settingsApi = api();
    const picker = {
      pickBackupDestination: vi
        .fn()
        .mockResolvedValueOnce(null)
        .mockResolvedValueOnce('C:/chosen.tlbackup'),
      pickBackupToRestore: vi.fn(),
    };
    renderSettings(settingsApi, picker);
    const user = userEvent.setup();
    await screen.findByRole('button', { name: 'Create backup' });
    await user.click(screen.getByRole('button', { name: 'Create backup' }));
    expect(
      screen.getByRole('dialog').getElementsByTagName('button')[0],
    ).toHaveFocus();
    expect(screen.getByRole('dialog')).toHaveTextContent(
      'Estimated backup size: 2 KB.',
    );
    await user.click(
      screen.getByRole('button', { name: 'Choose .tlbackup location' }),
    );
    expect(settingsApi.createLocalBackup).not.toHaveBeenCalled();
    await user.click(screen.getByRole('button', { name: 'Create backup' }));
    await user.click(
      screen.getByRole('button', { name: 'Choose .tlbackup location' }),
    );
    await waitFor(() =>
      expect(settingsApi.createLocalBackup).toHaveBeenCalledWith(
        'C:/chosen.tlbackup',
      ),
    );
    expect(
      screen.getByRole('status', { name: 'Settings operation status' }),
    ).toHaveTextContent('Backup created safely.');
  });

  it('keeps restore selection private, requires confirmation, and uses only the injected restart boundary', async () => {
    const settingsApi = api();
    const picker = {
      pickBackupDestination: vi.fn(),
      pickBackupToRestore: vi.fn().mockResolvedValue('C:/private.tlbackup'),
    };
    renderSettings(settingsApi, picker);
    const user = userEvent.setup();
    await user.click(
      await screen.findByRole('button', { name: 'Restore backup' }),
    );
    expect(screen.getByRole('dialog')).not.toHaveTextContent(
      'C:/private.tlbackup',
    );
    expect(settingsApi.restoreLocalBackup).not.toHaveBeenCalled();
    await user.click(
      screen.getByRole('button', { name: 'Verify and prepare restore' }),
    );
    await waitFor(() =>
      expect(settingsApi.restoreLocalBackup).toHaveBeenCalledWith(
        'C:/private.tlbackup',
      ),
    );
    expect(
      screen.getByRole('status', { name: 'Settings operation status' }),
    ).toHaveTextContent('Reconnect an AI service');
    await user.click(screen.getByRole('button', { name: 'Restart now' }));
    expect(settingsApi.restartApplication).toHaveBeenCalledOnce();
  });

  it('requires the exact clear phrase every time and preserves external backups in the warning', async () => {
    const settingsApi = api();
    renderSettings(settingsApi);
    const user = userEvent.setup();
    await user.click(
      await screen.findByRole('button', { name: 'Clear all data' }),
    );
    expect(screen.getByRole('dialog')).toHaveTextContent(
      'separately saved .tlbackup files are not deleted',
    );
    expect(
      within(screen.getByRole('dialog')).getAllByRole('listitem'),
    ).toHaveLength(6);
    const confirm = screen
      .getByRole('dialog')
      .getElementsByTagName('button')[1] as HTMLButtonElement;
    expect(confirm).toBeDisabled();
    await user.type(
      screen.getByRole('textbox'),
      'DELETE ALL TEXTBOOKLENS DATA',
    );
    await user.click(confirm);
    await waitFor(() =>
      expect(settingsApi.clearAllTextbookLensData).toHaveBeenCalledWith(
        'DELETE ALL TEXTBOOKLENS DATA',
      ),
    );
    expect(
      screen.getByRole('status', { name: 'Settings operation status' }),
    ).toHaveTextContent('Local data was cleared');
    await user.click(screen.getByRole('button', { name: 'Clear all data' }));
    expect(screen.getByRole('textbox')).toHaveValue('');
  });

  it('reports active work without exposing backend details and closes dialogs with Escape', async () => {
    const settingsApi = api();
    vi.mocked(settingsApi.createLocalBackup).mockRejectedValue({
      code: 'MAINTENANCE_BUSY',
      activeOperations: [{ kind: 'indexing', count: 2 }],
    });
    renderSettings(settingsApi, {
      pickBackupDestination: vi
        .fn()
        .mockResolvedValue('C:/never-shown.tlbackup'),
      pickBackupToRestore: vi.fn(),
    });
    const user = userEvent.setup();
    await user.click(
      await screen.findByRole('button', { name: 'Create backup' }),
    );
    await user.keyboard('{Escape}');
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Create backup' })).toHaveFocus();
    await user.click(screen.getByRole('button', { name: 'Create backup' }));
    await user.click(
      screen.getByRole('button', { name: 'Choose .tlbackup location' }),
    );
    await screen.findByRole('alert');
    expect(screen.getByRole('alert')).toHaveTextContent('indexing (2)');
    expect(screen.getByRole('alert')).not.toHaveTextContent(
      'C:/never-shown.tlbackup',
    );
  });
});
