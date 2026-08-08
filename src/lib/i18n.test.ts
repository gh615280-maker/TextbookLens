import { describe, expect, it } from 'vitest';

import { formatMessage, messageCatalogs } from './i18n';

function placeholders(message: string): string[] {
  return [...message.matchAll(/\{(\w+)\}/g)].map((match) => match[1]).sort();
}

const intentionallyShared = new Set([
  'language.zhCN',
  'language.zhTW',
  'language.en',
  'indexQuality.review.latex',
]);

describe('application message catalogs', () => {
  it('has exactly the same, nonempty keys and placeholders in every language', () => {
    const catalogs = messageCatalogs();
    const canonicalEntries = Object.entries(catalogs['zh-CN']);
    const canonicalKeys = canonicalEntries.map(([key]) => key).sort();

    for (const catalog of Object.values(catalogs)) {
      expect(Object.keys(catalog).sort()).toEqual(canonicalKeys);
      for (const [key, canonical] of canonicalEntries) {
        const message = catalog[key as keyof typeof catalog];
        expect(message.trim()).not.toHaveLength(0);
        expect(placeholders(message)).toEqual(placeholders(canonical));
      }
    }
  });

  it('does not silently reuse one language as a translation', () => {
    const catalogs = messageCatalogs();
    for (const key of Object.keys(catalogs.en)) {
      if (intentionallyShared.has(key)) continue;
      const typedKey = key as keyof typeof catalogs.en;
      expect(
        new Set([
          catalogs.en[typedKey],
          catalogs['zh-CN'][typedKey],
          catalogs['zh-TW'][typedKey],
        ]).size,
        key,
      ).toBeGreaterThan(1);
    }
  });
});

describe('formatMessage', () => {
  it('interpolates named values', () => {
    expect(
      formatMessage('zh-CN', 'example.greeting', { name: 'TextbookLens' }),
    ).toBe('你好，TextbookLens');
  });

  it('rejects unknown keys and missing placeholders instead of silently falling back', () => {
    expect(() => formatMessage('zh-CN', 'missing.key' as never)).toThrow(
      'Missing message: missing.key',
    );
    expect(() => formatMessage('en', 'example.greeting')).toThrow(
      'Missing value "name"',
    );
  });
});
