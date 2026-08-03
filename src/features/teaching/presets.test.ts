import { describe, expect, it } from 'vitest';

import { teachingPresets } from './presets';

describe('teaching presets', () => {
  it('provides five editable normal-text presets in each UI language', () => {
    expect(teachingPresets).toHaveLength(5);
    for (const preset of teachingPresets) {
      for (const language of ['zh-CN', 'zh-TW', 'en'] as const) {
        expect(preset.label[language].trim()).not.toBe('');
        expect(preset.instruction[language].trim()).not.toBe('');
      }
    }
    expect(JSON.stringify(teachingPresets)).not.toMatch(
      /persona|historical person/i,
    );
  });
});
