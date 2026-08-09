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

  it('uses a strict URI/namespace allowlist and ignores attacker-supplied IDs', () => {
    const input = `<h1 data-section-id="attacker" style="color:red">Safe</h1>
      <p data-block-id="attacker"><a href="jav&#x61;script:alert(1)">link text</a></p>
      <img src="data:image/svg+xml;base64,PHN2ZyBvbmxvYWQ9YWxlcnQoMSk+">
      <img src="data:text/html;base64,PHNjcmlwdD5iYWQ8L3NjcmlwdD4=">
      <img src="data:image/png;base64,iVBORw0KGgo=">
      <svg><foreignObject><p onclick="alert(1)">namespace text</p></foreignObject></svg>
      <math><mtext><img src=x onerror=alert(1)></mtext><mi>E</mi><mo>=</mo><mi>mc²</mi></math>`;

    const result = sanitizeDocxHtml(input, bookId);

    expect(result.html).not.toMatch(
      /attacker|style=|href=|javascript:|svg|foreignObject|onerror|onclick|data:text/iu,
    );
    expect(result.html).not.toContain('data:image/svg+xml');
    expect(result.html).toContain('data:image/png;base64,iVBORw0KGgo=');
    const sanitizedDocument = new DOMParser().parseFromString(
      result.html,
      'text/html',
    );
    expect(sanitizedDocument.querySelector('a')).toBeNull();
    expect(sanitizedDocument.querySelector('math')?.textContent).toContain(
      'E=mc²',
    );
    expect(result.sections[0]?.id).not.toBe('attacker');
    expect(result.sections[0]?.blocks[0]?.id).not.toBe('attacker');
  });

  it('measures DOCX offsets in Unicode code points and preserves canonical structure', () => {
    const result = sanitizeDocxHtml(
      '<h1>Emoji</h1><p>A😀B</p><ul><li>one</li></ul><table><tr><td>x</td><td>y</td></tr></table><figcaption>图 1 caption</figcaption><p>E = mc²</p>',
      bookId,
    );
    const blocks = result.sections[0]!.blocks;
    const emoji = blocks.find((block) => block.plainText === 'A😀B')!;

    expect(emoji.locator).toMatchObject({ startOffset: 0, endOffset: 3 });
    expect(blocks.map((block) => block.kind)).toEqual([
      'heading',
      'paragraph',
      'list',
      'table',
      'caption',
      'equation',
    ]);
    expect(blocks.at(-1)?.plainText).toBe('E = mc²');
  });
});
