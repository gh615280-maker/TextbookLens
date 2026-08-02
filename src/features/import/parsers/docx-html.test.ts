import { describe, expect, it } from 'vitest';

import { sanitizeDocxHtml } from './docx-html';

const bookId = '3f7080e5-2dfe-4ec9-ac5d-7e16176fe03c';

describe('sanitizeDocxHtml', () => {
  it('removes executable and external content while assigning deterministic IDs', () => {
    const input = `<h1 onclick="alert(1)">Chapter one</h1><p>Safe paragraph</p>
      <img src="https://evil.invalid/p.png"><link rel="stylesheet" href="https://evil.invalid/a.css">
      <script>window.bad = true</script><table><tr><td>x</td><td>y</td></tr></table><p><a href="javascript:alert(1)">bad</a></p>`;

    const first = sanitizeDocxHtml(input, bookId);
    const second = sanitizeDocxHtml(input, bookId);

    expect(first.html).toBe(second.html);
    expect(first.html).toContain('data-section-id=');
    expect(first.html).toContain('data-block-id=');
    expect(first.html).not.toMatch(
      /script|onclick|https:\/\/evil|javascript:/iu,
    );
    expect(first.sections).toHaveLength(1);
    expect(first.sections[0]?.blocks.map((block) => block.plainText)).toEqual([
      'Chapter one',
      'Safe paragraph',
      'x\ty',
      'bad',
    ]);
  });
});
