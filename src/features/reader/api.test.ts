import { describe, expect, it, vi } from 'vitest';

import { TauriReaderApi } from './api';

const state = vi.hoisted(() => ({ response: [] as unknown[] }));

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(async () => state.response),
}));

describe('TauriReaderApi annotation markers', () => {
  it('preserves tagged text and region anchors plus safe marker metadata', async () => {
    const selection = {
      locator: {
        format: 'pdf' as const,
        startPage: 1,
        endPage: 1,
        rectsByPage: null,
      },
      quote: { exact: 'text', prefix: '', suffix: '' },
      sectionId: 'section',
    };
    state.response = [
      {
        id: 'text',
        kind: 'note',
        conversationId: null,
        accessibilityLabel: 'View personal note marker',
        answer: 'must-not-cross-marker-dto',
        anchor: { kind: 'text', selection },
        relocationStatus: 'primary',
      },
      {
        id: 'region',
        kind: 'ai_conversation',
        conversationId: 'conversation',
        accessibilityLabel: 'View AI conversation marker',
        anchor: {
          kind: 'region',
          region: {
            locator: { format: 'pdf', page: 1 },
            rect: { x: 0.1, y: 0.1, width: 0.2, height: 0.2 },
            contentSha256: 'a'.repeat(64),
            textFallback: null,
          },
        },
        relocationStatus: 'primary',
      },
    ];

    const result = await new TauriReaderApi().listAnnotationMarkers('book');
    expect(result).toHaveLength(2);
    expect(result[0]).not.toHaveProperty('answer');
    expect(result[0].anchor).toEqual({ kind: 'text', selection });
    expect(result[1]).toMatchObject({
      conversationId: 'conversation',
      anchor: { kind: 'region' },
      relocationStatus: 'primary',
    });
  });

  it('searches editable question descriptions without searching textbook text', async () => {
    state.response = [
      {
        id: 'question-1',
        kind: 'ai_conversation',
        conversationId: 'conversation-1',
        accessibilityLabel: 'View AI conversation marker',
        anchor: {
          kind: 'text',
          selection: {
            locator: {
              format: 'pdf',
              startPage: 9,
              endPage: 9,
              rectsByPage: null,
            },
            quote: { exact: 'source', prefix: '', suffix: '' },
            sectionId: 'section',
          },
        },
        relocationStatus: 'primary',
        sequence: 1,
        summaryText: 'Compact operator explanation',
        revision: 1,
      },
    ];

    const result = await new TauriReaderApi().searchBook(
      'book',
      'operator',
      50,
      'question',
    );
    expect(result).toEqual([
      expect.objectContaining({
        snippet: 'Compact operator explanation',
        source: 'question',
        locator: expect.objectContaining({ startPage: 9 }),
      }),
    ]);
  });
});
