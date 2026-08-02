import { describe, expect, it } from 'vitest';

import { formatMessage } from './i18n';

describe('formatMessage', () => {
  it('interpolates named values', () => {
    expect(
      formatMessage('zh-CN', 'example.greeting', { name: 'TextbookLens' }),
    ).toBe('你好，TextbookLens');
  });

  it('falls back safely when a catalog key is unknown at runtime', () => {
    expect(formatMessage('zh-CN', 'missing.key' as never)).toBe('missing.key');
  });
});
