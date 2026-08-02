import { describe, expect, it } from 'vitest';

import { formatPersistedUtc, parsePersistedUtc } from './time';

describe('persisted UTC time', () => {
  it('strictly parses UTC ISO-8601 and formats a timezone rollover', () => {
    const value = '2026-08-01T18:30:45.123Z';

    expect(parsePersistedUtc(value).toISOString()).toBe(value);
    expect(
      formatPersistedUtc(value, {
        locale: 'en-GB',
        timeZone: 'Asia/Shanghai',
      }),
    ).toBe('02 Aug 2026, 02:30');
  });

  it.each([
    '',
    '2026-08-01',
    '2026-08-01T18:30:45',
    '2026-08-01T18:30:45+00:00',
    '2026-02-30T18:30:45Z',
    'not-a-date',
  ])('rejects invalid or non-UTC input: %s', (value) => {
    expect(() => parsePersistedUtc(value)).toThrow(RangeError);
  });

  it('uses deterministic injected locale and timezone options', () => {
    expect(
      formatPersistedUtc('2026-12-31T23:05:00Z', {
        locale: 'en-GB',
        timeZone: 'UTC',
      }),
    ).toBe('31 Dec 2026, 23:05');
  });
});
