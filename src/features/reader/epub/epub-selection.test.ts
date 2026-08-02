import { describe, expect, it } from 'vitest';
import { sanitizeEpubDocument, snapshotEpubRange } from './epub-selection';
import { recoverEpubCfi } from './epub-markers';

describe('EPUB selection security and CFI recovery', () => {
  it('creates an immutable CFI snapshot and strips executable/external content', () => {
    const document = new DOMParser().parseFromString(
      '<body><p>hello <b>world</b></p><script>throw 1</script><a href="https://bad.test">bad</a><img src="images/a.png" onerror="x()"></body>',
      'text/html',
    );
    sanitizeEpubDocument(document);
    expect(document.querySelector('script')).toBeNull();
    expect(document.querySelector('a')?.hasAttribute('href')).toBe(false);
    expect(document.querySelector('img')?.getAttribute('src')).toBe(
      'images/a.png',
    );
    const text = document.querySelector('b')!.firstChild!;
    const range = document.createRange();
    range.selectNodeContents(text);
    expect(snapshotEpubRange(range, 'spine-0', 'epubcfi(/6/4)')).toMatchObject({
      text: 'world',
      cfi: 'epubcfi(/6/4)',
    });
  });
  it('recovers only a unique quote within the supplied spine section', () => {
    const document = new DOMParser().parseFromString(
      '<body><p>prefix target suffix</p></body>',
      'text/html',
    );
    expect(
      recoverEpubCfi(
        { document, cfiFromElement: () => 'epubcfi(/6/4)' },
        { exact: 'target', prefix: 'prefix ', suffix: ' suffix' },
      ),
    ).toBe('epubcfi(/6/4)');
  });
});
