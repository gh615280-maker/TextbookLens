import AxeBuilder from '@axe-core/playwright';
import { expect, test, type Page } from '@playwright/test';

const BOOK_ID = '15150000-0000-4000-8000-000000000001';
const RUN_ID = '15150000-0000-4000-8000-000000000002';
const PAGE_ID = '15150000-0000-4000-8000-000000000003';
const BLOCK_ID = '15150000-0000-4000-8000-000000000004';
const CORRECTION_ID = '15150000-0000-4000-8000-000000000005';
const FIXED_TIME = '2026-08-08T00:00:00.000Z';

for (const decision of [
  ['keep', 'Keep correction'],
  ['accept', 'Accept current value'],
  ['compare', 'Resolve with compared value'],
] as const) {
  test(`M: ${decision[0]} is an explicit correction-conflict decision`, async ({
    page,
  }) => {
    const externalRequests = await installCorrectionMock(page);
    await page.goto(`/books/${BOOK_ID}/index-quality/${RUN_ID}`);

    await expect(
      page.getByRole('heading', { name: 'Review page 1' }),
    ).toBeVisible();
    await expect(
      page.getByText('Correction conflict needs a decision.'),
    ).toBeVisible();
    await expect(page.getByLabel('Editable value')).toHaveValue(
      'Synthetic user correction',
    );

    await page.getByRole('button', { name: decision[1] }).click();
    await expect
      .poll(() =>
        page.evaluate(
          () =>
            (
              window as unknown as {
                __p15t5CorrectionDecisions: string[];
              }
            ).__p15t5CorrectionDecisions,
        ),
      )
      .toEqual([decision[0]]);

    const axe = await new AxeBuilder({ page })
      .disableRules(['color-contrast'])
      .analyze();
    expect(
      axe.violations.filter((violation) =>
        ['critical', 'serious'].includes(violation.impact ?? ''),
      ),
    ).toEqual([]);
    expect(externalRequests).toEqual([]);
  });
}

async function installCorrectionMock(page: Page): Promise<string[]> {
  const externalRequests: string[] = [];
  await page.route('**/*', async (route) => {
    const url = new URL(route.request().url());
    if (
      url.protocol === 'data:' ||
      url.protocol === 'blob:' ||
      url.hostname === '127.0.0.1' ||
      url.hostname === 'localhost'
    ) {
      await route.continue();
      return;
    }
    externalRequests.push(url.origin);
    await route.abort('blockedbyclient');
  });

  await page.addInitScript(
    ({ bookId, runId, pageId, blockId, correctionId, fixedTime }) => {
      const decisions: string[] = [];
      const settings = {
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
      const correction = () => ({
        id: correctionId,
        targetBlockId: blockId,
        region: { x: 0.1, y: 0.1, width: 0.8, height: 0.2 },
        valueKind: 'text',
        originalValue: 'Synthetic provider transcription',
        correctedValue: 'Synthetic user correction',
        conflictState: 'conflict',
        revision: 2,
        updatedAt: fixedTime,
      });
      const review = () => ({
        id: pageId,
        runId,
        bookId,
        pageNumber: 1,
        qualityReason: 'very_low_text_coverage',
        status: 'needs_review',
        reviewReason: 'text_contradiction',
        safeError: null,
        contentVersion: 3,
        blocks: [
          {
            id: blockId,
            ordinal: 0,
            kind: 'paragraph',
            source: 'ai_transcribed',
            plainText: 'Synthetic provider transcription',
            latex: null,
            tableCells: null,
            visualDescription: null,
            bounds: { x: 0.1, y: 0.1, width: 0.8, height: 0.2 },
            contentVersion: 3,
          },
        ],
        corrections: decisions.length === 0 ? [correction()] : [],
        updatedAt: fixedTime,
      });
      const target = window as unknown as {
        __p15t5CorrectionDecisions: string[];
        __TAURI_INTERNALS__: {
          invoke(
            command: string,
            payload?: Record<string, unknown>,
          ): Promise<unknown>;
        };
      };
      target.__p15t5CorrectionDecisions = decisions;
      target.__TAURI_INTERNALS__ = {
        async invoke(command, payload = {}) {
          switch (command) {
            case 'get_app_settings':
            case 'initialize_ui_language':
              return settings;
            case 'get_onboarding_state':
              return {
                step: 'ready',
                selectedBook: null,
                hasReadyBook: true,
                learningProfileConnected: false,
                visionProfileConnected: false,
                localTextQuality: 'unreliable',
                canSkipOnboarding: true,
              };
            case 'get_index_run_aggregate':
              return {
                runId,
                bookId,
                controlStatus: 'paused',
                aggregateStatus: 'needs_review',
                pages: {
                  total: 1,
                  notRequired: 0,
                  queued: 0,
                  rendering: 0,
                  sending: 0,
                  parsing: 0,
                  validating: 0,
                  indexed: 0,
                  needsReview: 1,
                  failed: 0,
                  cancelled: 0,
                },
                updatedAt: fixedTime,
              };
            case 'list_index_page_reviews':
              return [review()];
            case 'read_book_source':
              throw new Error(
                'synthetic local preview intentionally unavailable',
              );
            case 'resolve_index_correction_conflict': {
              const request = payload.request as
                { decision?: string } | undefined;
              if (!request?.decision)
                throw new Error('missing explicit decision');
              decisions.push(request.decision);
              return request.decision === 'accept' ? null : correction();
            }
            default:
              throw new Error(`Unexpected P15T5 IPC command: ${command}`);
          }
        },
      };
    },
    {
      bookId: BOOK_ID,
      runId: RUN_ID,
      pageId: PAGE_ID,
      blockId: BLOCK_ID,
      correctionId: CORRECTION_ID,
      fixedTime: FIXED_TIME,
    },
  );
  return externalRequests;
}
