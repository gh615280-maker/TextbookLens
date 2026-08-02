import { describe, expect, it } from 'vitest';

import { createTextQuote, findTextQuote } from './text-quote';

describe('text quotes', () => {
  it('creates an exact quote with code-point bounded context', () => {
    const prefix = '😀'.repeat(65);
    const suffix = '中'.repeat(65);
    expect(createTextQuote(`${prefix}target${suffix}`, 65, 71)).toEqual({
      exact: 'target',
      prefix: '😀'.repeat(64),
      suffix: '中'.repeat(64),
    });
  });

  it('preserves emoji boundaries when creating and finding quotes', () => {
    const text = 'before 😀 selected 中 after';
    const quote = createTextQuote(text, 9, 19);

    expect(quote).toEqual({
      exact: 'selected 中',
      prefix: 'before 😀 ',
      suffix: ' after',
    });
    expect(findTextQuote(text, quote)).toEqual({
      startCp: 9,
      endCp: 19,
      startUtf16: 10,
      endUtf16: 20,
      confidence: 'exact_unique',
    });
  });

  it('returns a unique exact occurrence without requiring context', () => {
    expect(
      findTextQuote('changed prefix selected changed suffix', {
        exact: 'selected',
        prefix: 'old prefix',
        suffix: 'old suffix',
      }),
    ).toMatchObject({ startCp: 15, endCp: 23, confidence: 'exact_unique' });
  });

  it('uses both contexts to resolve repeated exact text', () => {
    const text = 'one before target after one other target elsewhere';
    expect(
      findTextQuote(text, {
        exact: 'target',
        prefix: 'before ',
        suffix: ' after',
      }),
    ).toMatchObject({ startCp: 11, endCp: 17, confidence: 'context_unique' });
  });

  it('prefers a matching suffix before a matching prefix', () => {
    const text = 'prefix target wrong and wrong target suffix';
    expect(
      findTextQuote(text, {
        exact: 'target',
        prefix: 'prefix ',
        suffix: ' suffix',
      }),
    ).toMatchObject({ startCp: 30, confidence: 'context_unique' });
  });

  it('rejects repeated text with equally reliable context', () => {
    const quote = { exact: 'target', prefix: 'same ', suffix: ' tail' };
    expect(
      findTextQuote('same target tail and same target tail', quote),
    ).toBeNull();
  });

  it('does not match altered text or reach beyond its supplied scope', () => {
    expect(
      findTextQuote('page one target', {
        exact: 'target',
        prefix: '',
        suffix: '',
      }),
    ).toMatchObject({ confidence: 'exact_unique' });
    expect(
      findTextQuote('page one', { exact: 'target', prefix: '', suffix: '' }),
    ).toBeNull();
    expect(
      findTextQuote('targetx', { exact: 'target ', prefix: '', suffix: '' }),
    ).toBeNull();
  });

  it('round-trips quote ranges across representative Unicode text', () => {
    for (const text of [
      'ASCII target text',
      '中😀 target café',
      'a\u0308 target',
    ]) {
      const points = Array.from(text);
      const startCp = points.indexOf('t');
      const endCp = startCp + 'target'.length;
      const quote = createTextQuote(text, startCp, endCp);
      expect(findTextQuote(text, quote)).toMatchObject({
        startCp,
        endCp,
        confidence: 'exact_unique',
      });
    }
  });
});
