import { describe, expect, it } from 'vitest';

import { detectWindowsLanguage } from './locale';

describe('detectWindowsLanguage', () => {
  it.each([
    [['zh-Hant'], 'zh-TW'],
    [['zh-TW'], 'zh-TW'],
    [['zh-HK'], 'zh-TW'],
    [['zh-MO'], 'zh-TW'],
    [['zh-Hans-CN'], 'zh-CN'],
    [['zh'], 'zh-CN'],
    [['fr-FR', 'zh-CN'], 'zh-CN'],
    [['en-US', 'de-DE'], 'en'],
  ] as const)('maps %j to %s', (languages, expected) => {
    expect(detectWindowsLanguage(languages)).toBe(expected);
  });
});
