const UTC_ISO_PATTERN =
  /^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2}):(\d{2})(?:\.(\d{1,9}))?Z$/;

export interface TimeFormatOptions {
  locale?: string;
  timeZone?: string;
}

export function parsePersistedUtc(value: string): Date {
  const match = UTC_ISO_PATTERN.exec(value);
  if (!match) throw new RangeError('Persisted timestamp must be UTC ISO-8601');

  const [, yearText, monthText, dayText, hourText, minuteText, secondText] =
    match;
  const fraction = match[7] ?? '';
  const year = Number(yearText);
  const month = Number(monthText);
  const day = Number(dayText);
  const hour = Number(hourText);
  const minute = Number(minuteText);
  const second = Number(secondText);
  const millisecond = Number(fraction.padEnd(3, '0').slice(0, 3));
  const date = new Date(0);
  date.setUTCFullYear(year, month - 1, day);
  date.setUTCHours(hour, minute, second, millisecond);
  if (
    date.getUTCFullYear() !== year ||
    date.getUTCMonth() !== month - 1 ||
    date.getUTCDate() !== day ||
    date.getUTCHours() !== hour ||
    date.getUTCMinutes() !== minute ||
    date.getUTCSeconds() !== second
  ) {
    throw new RangeError('Persisted timestamp is not a real UTC instant');
  }
  return date;
}

export function formatPersistedUtc(
  value: string,
  options: TimeFormatOptions = {},
): string {
  return new Intl.DateTimeFormat(options.locale, {
    year: 'numeric',
    month: 'short',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
    timeZone: options.timeZone,
    hourCycle: 'h23',
  }).format(parsePersistedUtc(value));
}
