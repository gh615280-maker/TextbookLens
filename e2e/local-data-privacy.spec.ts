import AxeBuilder from '@axe-core/playwright';
import { expect, test, type Page } from '@playwright/test';

const privateBackupPath = 'C:/isolated/private-checkpoint.tlbackup';

async function installStrictMaintenanceMock(page: Page) {
  await page.addInitScript((backupPath) => {
    type BackupMode = 'success' | 'busy' | 'pending';
    type MockConfiguration = {
      backupMode?: BackupMode;
      saveSelections?: Array<string | null>;
      restoreSelection?: string | null;
      activeOperations?: Array<{ kind: string; count: number }>;
    };
    type Snapshot = {
      calls: string[];
      backupCalls: number;
      clearCalls: number;
      restartCalls: number;
    };
    type TestWindow = Window & {
      __TAURI_INTERNALS__: {
        invoke(
          command: string,
          payload?: Record<string, unknown>,
        ): Promise<unknown>;
      };
      __p13Configure(configuration: MockConfiguration): void;
      __p13Snapshot(): Snapshot;
    };

    const testWindow = window as TestWindow;
    const calls: string[] = [];
    const state = {
      backupMode: 'success' as BackupMode,
      saveSelections: [backupPath] as Array<string | null>,
      restoreSelection: backupPath as string | null,
      activeOperations: [] as Array<{ kind: string; count: number }>,
      backupCalls: 0,
      clearCalls: 0,
      restartCalls: 0,
      cleared: false,
    };
    const appSettings = {
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
    const onboardingState = () => ({
      step: state.cleared ? 'book' : 'ready',
      selectedBook: null,
      hasReadyBook: false,
      learningProfileConnected: false,
      visionProfileConnected: false,
      localTextQuality: 'not_ready',
      canSkipOnboarding: !state.cleared,
    });
    testWindow.__p13Configure = (configuration) => {
      if (configuration.backupMode) state.backupMode = configuration.backupMode;
      if (configuration.saveSelections)
        state.saveSelections = [...configuration.saveSelections];
      if (configuration.restoreSelection !== undefined)
        state.restoreSelection = configuration.restoreSelection;
      if (configuration.activeOperations)
        state.activeOperations = [...configuration.activeOperations];
    };
    testWindow.__p13Snapshot = () => ({
      calls: [...calls],
      backupCalls: state.backupCalls,
      clearCalls: state.clearCalls,
      restartCalls: state.restartCalls,
    });
    testWindow.__TAURI_INTERNALS__ = {
      async invoke(command) {
        calls.push(command);
        switch (command) {
          case 'get_app_settings':
            return appSettings;
          case 'get_reader_settings':
            return {
              theme: 'system',
              fontScale: 1,
              lineHeight: 1.5,
              readerWidth: 72,
              pdfZoom: 1,
            };
          case 'get_maintenance_status':
            return {
              code:
                state.activeOperations.length > 0
                  ? 'NORMAL_OPERATIONS_ACTIVE'
                  : 'MAINTENANCE_AVAILABLE',
              activeOperations: state.activeOperations,
            };
          case 'get_storage_usage':
            return {
              totalBytes: 8192,
              totalFileCount: 6,
              categories: [
                { category: 'source', bytes: 4096, fileCount: 3 },
                { category: 'derived', bytes: 1024, fileCount: 1 },
                { category: 'index', bytes: 1024, fileCount: 1 },
                { category: 'database', bytes: 2048, fileCount: 1 },
              ],
            };
          case 'open_app_data_directory':
            return undefined;
          case 'plugin:dialog|save':
            return state.saveSelections.shift() ?? null;
          case 'create_local_backup':
            state.backupCalls += 1;
            if (state.backupMode === 'busy') {
              throw {
                code: 'MAINTENANCE_BUSY',
                activeOperations: state.activeOperations,
              };
            }
            if (state.backupMode === 'pending') {
              return new Promise(() => undefined);
            }
            return { formatVersion: 1, archiveBytes: 8192, entryCount: 7 };
          case 'plugin:dialog|open':
            return state.restoreSelection;
          case 'restore_local_backup':
            return {
              status: 'RESTORE_READY_TO_RESTART',
              restartRequired: true,
              aiConfigurationRequired: true,
            };
          case 'clear_all_textbooklens_data':
            state.clearCalls += 1;
            if (state.clearCalls === 1) {
              return {
                status: 'CLEAR_CREDENTIAL_CLEANUP_REQUIRED',
                restartRequired: false,
              };
            }
            state.cleared = true;
            return {
              status: 'CLEAR_READY_TO_RESTART',
              restartRequired: true,
            };
          case 'restart_application':
            state.restartCalls += 1;
            if (state.cleared) {
              setTimeout(() => {
                history.pushState({}, '', '/onboarding');
                dispatchEvent(new PopStateEvent('popstate'));
              }, 0);
            }
            return undefined;
          case 'get_onboarding_state':
            return onboardingState();
          case 'list_books':
            return [];
          default:
            throw new Error(`Unexpected IPC command: ${command}`);
        }
      },
    };
  }, privateBackupPath);
}

test.beforeEach(async ({ page }) => {
  await installStrictMaintenanceMock(page);
});

test('P: settings backup and restore stay path-free, confirmed, keyboard-safe, and credential-aware', async ({
  page,
}) => {
  const externalRequests = collectExternalRequests(page);
  await page.goto('/settings');
  await configure(page, { saveSelections: [null, privateBackupPath] });

  await expect(page.getByRole('heading', { level: 2 })).toHaveText([
    'Reading experience',
    'Data and privacy',
    'About',
  ]);
  await expect(
    page.getByRole('heading', { name: 'Local storage' }),
  ).toBeVisible();
  await expect(page.getByText('8 KB')).toBeVisible();
  await page.getByRole('button', { name: 'Open data directory' }).click();

  const backupButton = page.getByRole('button', { name: 'Create backup' });
  await backupButton.click();
  const backupDialog = page.getByRole('dialog', {
    name: 'Create local backup',
  });
  await expect(backupDialog).toContainText('Estimated backup size: 8 KB.');
  await expect(backupDialog).toContainText('textbook copies');
  await expect(backupDialog).toContainText('does not include API keys');
  await expect(
    backupDialog.getByRole('button', { name: 'Cancel' }),
  ).toBeFocused();
  await page.keyboard.press('Shift+Tab');
  await expect(
    backupDialog.getByRole('button', { name: 'Choose .tlbackup location' }),
  ).toBeFocused();
  await page.keyboard.press('Escape');
  await expect(backupDialog).toHaveCount(0);
  await expect(backupButton).toBeFocused();

  await backupButton.click();
  await page.getByRole('button', { name: 'Choose .tlbackup location' }).click();
  expect((await snapshot(page)).backupCalls).toBe(0);
  await page.getByRole('button', { name: 'Choose .tlbackup location' }).click();
  await expect(
    page.getByRole('status', { name: 'Settings operation status' }),
  ).toContainText('Backup created safely.');

  await page.getByRole('button', { name: 'Restore backup' }).click();
  const restoreDialog = page.getByRole('dialog', { name: 'Restore backup?' });
  await expect(restoreDialog).not.toContainText(privateBackupPath);
  await expect(restoreDialog).toContainText('current data is kept');
  await restoreDialog
    .getByRole('button', { name: 'Verify and prepare restore' })
    .click();
  const status = page.getByRole('status', {
    name: 'Settings operation status',
  });
  await expect(status).toContainText('Reconnect an AI service');
  await expect(
    status.getByRole('button', { name: 'Restart now' }),
  ).toBeVisible();
  await status.getByRole('button', { name: 'Restart now' }).click();

  await expect(page.locator('body')).not.toContainText(privateBackupPath);
  expect((await snapshot(page)).restartCalls).toBe(1);
  expect(externalRequests).toEqual([]);
});

test('Q: busy failure and credential retry never fake success, and clear restarts to onboarding', async ({
  page,
}) => {
  await page.goto('/settings');
  await configure(page, {
    backupMode: 'busy',
    saveSelections: [privateBackupPath],
    activeOperations: [
      { kind: 'indexing', count: 99 },
      { kind: 'learning', count: 2 },
    ],
  });
  await page.getByRole('button', { name: 'Refresh' }).click();
  await expect(page.getByText(/indexing \(99\).*learning \(2\)/)).toBeVisible();

  await page.getByRole('button', { name: 'Create backup' }).click();
  await page.getByRole('button', { name: 'Choose .tlbackup location' }).click();
  await expect(page.getByRole('alert')).toContainText('indexing (99)');
  await expect(page.getByRole('alert')).not.toContainText(privateBackupPath);
  await page.keyboard.press('Escape');
  await expect(
    page.getByRole('dialog', { name: 'Create local backup' }),
  ).toHaveCount(0);

  const clearButton = page.getByRole('button', { name: 'Clear all data' });
  await clearButton.click();
  let clearDialog = page.getByRole('dialog', {
    name: 'Clear all TextbookLens data?',
  });
  await expect(clearDialog.getByRole('listitem')).toHaveCount(6);
  await expect(clearDialog).toContainText('original files');
  await expect(clearDialog).toContainText('separately saved .tlbackup files');
  await page.keyboard.press('Escape');
  await expect(clearButton).toBeFocused();

  await clearButton.click();
  clearDialog = page.getByRole('dialog', {
    name: 'Clear all TextbookLens data?',
  });
  const confirmation = clearDialog.getByRole('textbox');
  const confirm = clearDialog.getByRole('button', { name: 'Clear all data' });
  await confirmation.fill('DELETE ALL TEXTBOOKLENS DATA ');
  await expect(confirm).toBeDisabled();
  await confirmation.fill('DELETE ALL TEXTBOOKLENS DATA');
  await confirm.click();
  await expect(
    page.getByRole('status', { name: 'Settings operation status' }),
  ).toContainText('credential cleanup still needs a retry');
  await expect(page.getByRole('button', { name: 'Restart now' })).toHaveCount(
    0,
  );

  await clearButton.click();
  clearDialog = page.getByRole('dialog', {
    name: 'Clear all TextbookLens data?',
  });
  await expect(clearDialog.getByRole('textbox')).toHaveValue('');
  await clearDialog.getByRole('textbox').fill('DELETE ALL TEXTBOOKLENS DATA');
  await clearDialog.getByRole('button', { name: 'Clear all data' }).click();
  await expect(
    page.getByRole('status', { name: 'Settings operation status' }),
  ).toContainText('Local data was cleared');
  await page.getByRole('button', { name: 'Restart now' }).click();
  await expect(page).toHaveURL(/\/onboarding$/);
  await expect(page.getByRole('heading', { level: 1 })).toHaveText(
    'Get started',
  );
  expect(await snapshot(page)).toMatchObject({
    clearCalls: 2,
    restartCalls: 1,
  });
});

test('active maintenance survives Escape and navigation at narrow 200% high-contrast reduced-motion settings', async ({
  page,
}) => {
  const externalRequests = collectExternalRequests(page);
  await page.emulateMedia({ forcedColors: 'active', reducedMotion: 'reduce' });
  await page.setViewportSize({ width: 640, height: 720 });
  await page.goto('/settings');
  await configure(page, {
    backupMode: 'pending',
    saveSelections: [privateBackupPath],
  });
  await page.evaluate(() => {
    document.documentElement.style.zoom = '2';
  });
  expect(
    await page.evaluate(
      () =>
        matchMedia('(forced-colors: active)').matches &&
        matchMedia('(prefers-reduced-motion: reduce)').matches,
    ),
  ).toBe(true);
  expect(
    (await new AxeBuilder({ page }).exclude('.katex').analyze()).violations,
  ).toEqual([]);

  await page.getByRole('button', { name: 'Create backup' }).click();
  await page.getByRole('button', { name: 'Choose .tlbackup location' }).click();
  const dialog = page.getByRole('dialog', { name: 'Create local backup' });
  await expect(dialog).toContainText('Creating backup…');
  await page.keyboard.press('Escape');
  await expect(dialog).toBeVisible();
  await page.evaluate(() => {
    history.pushState({}, '', '/library');
    dispatchEvent(new PopStateEvent('popstate'));
  });
  await expect(page).toHaveURL(/\/library$/);

  const state = await snapshot(page);
  expect(state.backupCalls).toBe(1);
  expect(state.calls.some((command) => /cancel|unlock/u.test(command))).toBe(
    false,
  );
  expect(externalRequests).toEqual([]);
});

async function configure(
  page: Page,
  configuration: {
    backupMode?: 'success' | 'busy' | 'pending';
    saveSelections?: Array<string | null>;
    restoreSelection?: string | null;
    activeOperations?: Array<{ kind: string; count: number }>;
  },
) {
  await page.evaluate((value) => {
    (
      window as unknown as {
        __p13Configure(configuration: typeof value): void;
      }
    ).__p13Configure(value);
  }, configuration);
}

async function snapshot(page: Page) {
  return page.evaluate(() =>
    (
      window as unknown as {
        __p13Snapshot(): {
          calls: string[];
          backupCalls: number;
          clearCalls: number;
          restartCalls: number;
        };
      }
    ).__p13Snapshot(),
  );
}

function collectExternalRequests(page: Page) {
  const requests: string[] = [];
  page.on('request', (request) => {
    const url = new URL(request.url());
    if (
      !['127.0.0.1', 'localhost'].includes(url.hostname) &&
      !['about:', 'data:', 'blob:'].includes(url.protocol)
    ) {
      requests.push(`${url.protocol}//${url.host}`);
    }
  });
  return requests;
}
