import { toUtf16Offset } from './code-points';
import type { QuoteMatch, TextQuote } from './types';

const CONTEXT_LIMIT = 64;

function codePoints(text: string): string[] {
  return Array.from(text);
}

function occurrences(text: string, exact: string): number[] {
  const indexes: number[] = [];
  let index = text.indexOf(exact);
  while (index !== -1) {
    indexes.push(index);
    index = text.indexOf(exact, index + Math.max(exact.length, 1));
  }
  return indexes;
}

function contextScore(
  text: string,
  startUtf16: number,
  exact: string,
  quote: TextQuote,
): [number, number] {
  const suffixMatches =
    quote.suffix.length > 0 &&
    text.startsWith(quote.suffix, startUtf16 + exact.length);
  const prefixMatches =
    quote.prefix.length > 0 && text.slice(0, startUtf16).endsWith(quote.prefix);
  return [Number(suffixMatches), Number(prefixMatches)];
}

export function createTextQuote(
  text: string,
  startCp: number,
  endCp: number,
): TextQuote {
  const points = codePoints(text);
  if (
    !Number.isInteger(startCp) ||
    !Number.isInteger(endCp) ||
    startCp < 0 ||
    endCp > points.length ||
    startCp >= endCp
  ) {
    throw new RangeError(
      'Quote offsets must describe a non-empty code-point range within the text',
    );
  }
  return {
    exact: points.slice(startCp, endCp).join(''),
    prefix: points
      .slice(Math.max(0, startCp - CONTEXT_LIMIT), startCp)
      .join(''),
    suffix: points.slice(endCp, endCp + CONTEXT_LIMIT).join(''),
  };
}

export function findTextQuote(
  text: string,
  quote: TextQuote,
): QuoteMatch | null {
  if (quote.exact.length === 0) {
    return null;
  }
  const matches = occurrences(text, quote.exact);
  if (matches.length === 0) {
    return null;
  }
  let chosen = matches;
  let confidence: QuoteMatch['confidence'] = 'exact_unique';
  if (matches.length > 1) {
    const scored = matches.map((startUtf16) => ({
      startUtf16,
      score: contextScore(text, startUtf16, quote.exact, quote),
    }));
    const bestScore = scored.reduce<[number, number]>(
      (best, { score }) =>
        score[0] > best[0] || (score[0] === best[0] && score[1] > best[1])
          ? score
          : best,
      [0, 0],
    );
    chosen = scored
      .filter(
        ({ score }) => score[0] === bestScore[0] && score[1] === bestScore[1],
      )
      .map(({ startUtf16 }) => startUtf16);
    if ((bestScore[0] === 0 && bestScore[1] === 0) || chosen.length !== 1) {
      return null;
    }
    confidence = 'context_unique';
  }
  const startUtf16 = chosen[0];
  const endUtf16 = startUtf16 + quote.exact.length;
  return {
    startCp: codePoints(text.slice(0, startUtf16)).length,
    endCp: codePoints(text.slice(0, endUtf16)).length,
    startUtf16,
    endUtf16,
    confidence,
  };
}

export { toUtf16Offset };
