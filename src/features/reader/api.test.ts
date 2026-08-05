import { describe, expect, it, vi } from 'vitest';

import { TauriReaderApi } from './api';

const state = vi.hoisted(() => ({ response: [] as unknown[] }));

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(async () => state.response),
}));

describe('TauriReaderApi annotation markers', () => {
  it('unwraps tagged text anchors and keeps region anchors history-only until region adapters exist', async () => {
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
        anchor: { kind: 'text', selection },
        relocationStatus: 'primary',
      },
      {
        id: 'region',
        kind: 'ai_conversation',
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

    await expect(
      new TauriReaderApi().listAnnotationMarkers('book'),
    ).resolves.toEqual([
      {
        id: 'text',
        kind: 'note',
        anchor: selection,
        relocationStatus: 'primary',
      },
      {
        id: 'region',
        kind: 'ai_conversation',
        anchor: null,
        relocationStatus: 'unresolved',
      },
    ]);
  });
});
