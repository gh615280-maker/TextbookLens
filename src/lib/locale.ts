import type { UiLanguage } from './i18n';

const traditionalRegions = new Set(['TW', 'HK', 'MO']);

export function detectWindowsLanguage(
  languages: readonly string[],
): UiLanguage {
  for (const locale of languages) {
    const subtags = locale.toUpperCase().split(/[-_]/);
    if (subtags[0] !== 'ZH') continue;
    if (
      subtags.includes('HANT') ||
      subtags.some((subtag) => traditionalRegions.has(subtag))
    ) {
      return 'zh-TW';
    }
    return 'zh-CN';
  }
  return 'en';
}
