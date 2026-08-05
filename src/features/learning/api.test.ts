import { afterEach, describe, expect, it } from 'vitest';

import { clearMocks, installTauriMock } from '../../test/tauri-mock';
import { TauriLearningApi } from './api';
import type {
  PreparationSummary,
  PrepareLearningRequestMetadata,
  RegionCaptureMetadata,
} from './learning-contract';

const BOOK_ID = '11111111-1111-4111-8111-111111111111';
const SECTION_ID = '22222222-2222-4222-8222-222222222222';
const PROFILE_ID = '33333333-3333-4333-8333-333333333333';
const PREPARATION_ID = '44444444-4444-4444-8444-444444444444';
const OPERATION_TOKEN = '55555555-5555-4555-8555-555555555555';
const MODEL_ID = 'provider-model-id-sentinel';
const TEXTBOOK_SENTINEL = 'PRIVATE_TEXTBOOK_BODY_SENTINEL';

afterEach(() => clearMocks());

describe('TauriLearningApi', () => {
  it('validates and deeply freezes an exact safe preparation summary', async () => {
    const calls = installTauriMock((command) => {
      expect(command).toBe('prepare_learning_request');
      return safeSummary();
    });
    const api = new TauriLearningApi();
    const metadata = textMetadata();
    const summary = await api.prepare(metadata);

    expect(summary).toEqual(safeSummary());
    expect(Object.isFrozen(summary)).toBe(true);
    expect(Object.isFrozen(summary.riskFlags)).toBe(true);
    expect(calls).toHaveLength(1);
    const recorded = JSON.stringify(calls[0]?.payload);
    expect(recorded).not.toContain(TEXTBOOK_SENTINEL);
    expect(recorded).not.toContain(MODEL_ID);
    expect(recorded).not.toContain('contentSha256');
    expect(recorded).toContain('text_selection');
  });

  it('rejects unsafe metadata before IPC and exposes only a stable code', async () => {
    const calls = installTauriMock(() => safeSummary());
    const api = new TauriLearningApi();
    const unsafe = {
      ...textMetadata(),
      unexpectedPrompt: TEXTBOOK_SENTINEL,
    } as PrepareLearningRequestMetadata;
    await expect(api.prepare(unsafe)).rejects.toEqual({
      code: 'INVALID_INPUT',
    });
    expect(calls).toHaveLength(0);

    installTauriMock(() => {
      throw {
        code: 'CONTEXT_TOO_LARGE',
        message: TEXTBOOK_SENTINEL,
        nextStep: TEXTBOOK_SENTINEL,
        diagnosticId: null,
      };
    });
    await expect(api.prepare(textMetadata())).rejects.toEqual({
      code: 'CONTEXT_TOO_LARGE',
    });
  });

  it('hash-checks bounded capture bytes, records no image body, and zeroes buffers', async () => {
    const bytes = new TextEncoder().encode('IMAGE_BODY_SENTINEL');
    const hash = await sha256Hex(bytes);
    const calls = installTauriMock((command) => {
      expect(command).toBe('stage_region_capture');
      return undefined;
    });
    const api = new TauriLearningApi();
    await api.stageRegionCapture(
      captureMetadata(hash, bytes.byteLength),
      bytes,
    );

    expect([...bytes].every((value) => value === 0)).toBe(true);
    expect(calls).toHaveLength(1);
    expect(calls[0]?.payload).toEqual({
      encodedByteLength: 'IMAGE_BODY_SENTINEL'.length,
    });
    const recorded = JSON.stringify(calls[0]?.payload);
    expect(recorded).not.toContain('IMAGE_BODY_SENTINEL');
    expect(recorded).not.toContain(hash);
    expect(recorded).not.toContain(MODEL_ID);
  });

  it('zeroes rejected capture bytes without invoking and rejects binding metadata changes', async () => {
    const calls = installTauriMock(() => undefined);
    const api = new TauriLearningApi();
    const bytes = new TextEncoder().encode('REJECTED_IMAGE_SENTINEL');
    const metadata = {
      ...captureMetadata('a'.repeat(64), bytes.byteLength),
      providerProfileId: 'not-a-uuid',
    } as RegionCaptureMetadata;
    await expect(api.stageRegionCapture(metadata, bytes)).rejects.toEqual({
      code: 'INVALID_INPUT',
    });
    expect([...bytes].every((value) => value === 0)).toBe(true);
    expect(calls).toHaveLength(0);

    const mismatch = new TextEncoder().encode('HASH_MISMATCH_SENTINEL');
    await expect(
      api.stageRegionCapture(
        captureMetadata('b'.repeat(64), mismatch.byteLength),
        mismatch,
      ),
    ).rejects.toEqual({ code: 'REQUEST_CONFLICT' });
    expect([...mismatch].every((value) => value === 0)).toBe(true);
    expect(calls).toHaveLength(0);
  });

  it('uses only preparation, authorization, staging and invalidation commands without events', async () => {
    const calls = installTauriMock((command) => {
      if (command === 'authorize_learning_request') return OPERATION_TOKEN;
      if (command === 'invalidate_learning_preparations') return 2;
      return undefined;
    });
    const api = new TauriLearningApi();
    await expect(api.authorize(PREPARATION_ID, 'allow')).resolves.toBe(
      OPERATION_TOKEN,
    );
    await api.discard(PREPARATION_ID);
    await expect(
      api.invalidate({
        reason: 'default_change',
        bookId: null,
        providerProfileId: null,
      }),
    ).resolves.toBe(2);

    expect(calls.map((call) => call.command)).toEqual([
      'authorize_learning_request',
      'discard_learning_preparation',
      'invalidate_learning_preparations',
    ]);
    expect(
      Object.keys(api).some((key) => key.toLowerCase().includes('event')),
    ).toBe(false);
    const recorded = JSON.stringify(calls);
    expect(recorded).not.toContain(TEXTBOOK_SENTINEL);
    expect(recorded).not.toContain(MODEL_ID);
  });

  it('rejects response DTO additions so raw source or internal IDs cannot enter state', async () => {
    installTauriMock(() => ({
      ...safeSummary(),
      rawSource: TEXTBOOK_SENTINEL,
      providerProfileId: PROFILE_ID,
    }));
    const api = new TauriLearningApi();
    await expect(api.prepare(textMetadata())).rejects.toEqual({
      code: 'DATABASE_ERROR',
    });

    installTauriMock(() => ({
      ...safeSummary(),
      riskFlags: ['image_send'],
      willSendImage: false,
    }));
    await expect(api.prepare(textMetadata())).rejects.toEqual({
      code: 'DATABASE_ERROR',
    });
  });
});

function textMetadata(): PrepareLearningRequestMetadata {
  return {
    bookId: BOOK_ID,
    sectionId: SECTION_ID,
    providerProfileId: PROFILE_ID,
    modelId: MODEL_ID,
    action: 'explain',
    contentKind: 'text_selection',
    anchor: {
      kind: 'text',
      selection: {
        locator: {
          format: 'pdf',
          startPage: 1,
          endPage: 1,
          rectsByPage: null,
        },
        quote: { exact: TEXTBOOK_SENTINEL, prefix: '', suffix: '' },
        sectionId: SECTION_ID,
      },
    },
    selectedText: TEXTBOOK_SENTINEL,
    question: null,
    targetLanguage: null,
  };
}

function safeSummary(): PreparationSummary {
  return {
    preparationId: PREPARATION_ID,
    providerDisplayName: 'OpenAI',
    profileDisplayName: 'Learning profile',
    modelDisplayName: 'Safe model display',
    estimatedInputTokens: 1200,
    sourceCount: 4,
    citationCount: 3,
    omittedSourceCount: 1,
    willSendImage: false,
    riskFlags: [],
    requiresBlockingConfirmation: false,
    expiresAt: '2026-08-05T12:05:00Z',
    actionCategory: 'explain',
  };
}

function captureMetadata(
  hash: string,
  encodedByteLength: number,
): RegionCaptureMetadata {
  return {
    preparationId: PREPARATION_ID,
    operationToken: OPERATION_TOKEN,
    bookId: BOOK_ID,
    providerProfileId: PROFILE_ID,
    modelId: MODEL_ID,
    anchorContentSha256: hash,
    captureSha256: hash,
    schemaVersion: 1,
    mimeType: 'image/png',
    width: 2,
    height: 2,
    decodedPixelCount: 4,
    encodedByteLength,
  };
}

async function sha256Hex(bytes: Uint8Array): Promise<string> {
  const digest = await crypto.subtle.digest(
    'SHA-256',
    Uint8Array.from(bytes).buffer,
  );
  return [...new Uint8Array(digest)]
    .map((value) => value.toString(16).padStart(2, '0'))
    .join('');
}
