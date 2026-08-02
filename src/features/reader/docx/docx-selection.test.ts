import { describe, expect, it } from 'vitest';
import { rangeFromDocxLocator, snapshotDocxRange } from './docx-selection';
import { recoverDocxRange } from './docx-markers';

describe('DOCX code-point selection', () => {
  it('converts nested emoji text to block-relative code points and back', () => {
    document.body.innerHTML = '<main><p data-section-id="s1" data-block-id="a">A<span>😀B</span></p><p data-section-id="s1" data-block-id="b">second</p></main>';
    const root = document.querySelector('main')!; const text = root.querySelector('span')!.firstChild!; const range = document.createRange(); range.setStart(text, 0); range.setEnd(text, 2);
    const snapshot = snapshotDocxRange(range, root); expect(snapshot?.locator).toMatchObject({ startBlockId: 'a', startOffset: 1, endOffset: 2 });
    expect(rangeFromDocxLocator(root, snapshot!.locator as never)?.toString()).toBe('😀');
  });
  it('rejects cross-section selections rather than guessing an anchor', () => { document.body.innerHTML = '<main><p data-section-id="a" data-block-id="a">one</p><p data-section-id="b" data-block-id="b">two</p></main>'; const root = document.querySelector('main')!; const range = document.createRange(); range.setStart(root.firstChild!.firstChild!, 0); range.setEnd(root.lastChild!.firstChild!, 3); expect(snapshotDocxRange(range, root)).toBeNull(); });
  it('falls back only to a unique quote in the recorded section', () => { document.body.innerHTML = '<main><p data-section-id="s" data-block-id="x">prefix target suffix</p><p data-section-id="other" data-block-id="y">prefix target suffix</p></main>'; const root = document.querySelector('main')!; expect(recoverDocxRange(root, 's', { exact: 'target', prefix: 'prefix ', suffix: ' suffix' })?.toString()).toBe('target'); });
});
