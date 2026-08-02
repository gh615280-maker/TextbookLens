export interface PdfTextItem {
  str: string;
  transform: readonly number[];
  width: number;
}

export interface PdfTextBlock {
  text: string;
}

const BASELINE_TOLERANCE = 3;
const PARAGRAPH_GAP = 30;

/** Converts PDF.js text items into reading-order paragraphs without geometry. */
export function normalizePdfPage(items: readonly PdfTextItem[]): PdfTextBlock[] {
  const lines: Array<{ y: number; items: PdfTextItem[] }> = [];

  for (const item of items) {
    if (!item.str.trim()) continue;
    const y = item.transform[5];
    if (typeof y !== 'number') continue;
    const line = lines.find((candidate) => Math.abs(candidate.y - y) <= BASELINE_TOLERANCE);
    if (line) line.items.push(item);
    else lines.push({ y, items: [item] });
  }

  const orderedLines = lines
    .sort((left, right) => right.y - left.y)
    .map((line) => ({
      y: line.y,
      text: normalizeWhitespace(
        line.items
          .sort((left, right) => (left.transform[4] ?? 0) - (right.transform[4] ?? 0))
          .map((item) => item.str)
          .join(' '),
      ),
    }))
    .filter((line) => line.text.length > 0);

  const blocks: PdfTextBlock[] = [];
  let previousY: number | undefined;
  for (const line of orderedLines) {
    if (previousY === undefined || previousY - line.y > PARAGRAPH_GAP) {
      blocks.push({ text: line.text });
    } else {
      const previous = blocks.at(-1);
      if (previous) previous.text = normalizeWhitespace(`${previous.text} ${line.text}`);
    }
    previousY = line.y;
  }
  return blocks;
}

export function normalizeWhitespace(value: string): string {
  return value.replace(/\s+/gu, ' ').trim();
}
