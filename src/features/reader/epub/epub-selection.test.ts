import { describe, expect, it } from 'vitest';
import { sanitizeEpubDocument, snapshotEpubRange } from './epub-selection';
import { recoverEpubCfi } from './epub-markers';

describe('EPUB selection security and CFI recovery', () => {
  it('captures the actual occurrence when a selected word repeats in a chapter', () => {
    const document = new DOMParser().parseFromString(
      '<body><p>first eigenvalue here</p><p>second eigenvalue there</p></body>',
      'text/html',
    );
    const node = document.querySelectorAll('p')[1]!.firstChild!;
    const range = document.createRange();
    range.setStart(node, 7);
    range.setEnd(node, 17);
    expect(
      snapshotEpubRange(range, 'section', 'epubcfi(/6/38!/4/4/1:7)')?.quote,
    ).toMatchObject({ exact: 'eigenvalue', suffix: ' there' });
  });
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

  it('confines restored archive assets and rejects remote URLs and refresh navigation', () => {
    const document = new DOMParser().parseFromString(
      '<html><head><meta http-equiv="refresh" content="0;url=https://outside.invalid"><meta data-textbooklens-csp http-equiv="Content-Security-Policy" content="default-src *"></head><body><img src="//outside.invalid/track"><img src="blob:http://localhost/synthetic-image"><a href=" javascript:alert(1)">link</a></body></html>',
      'text/html',
    );
    sanitizeEpubDocument(document);
    expect(document.querySelector('meta[http-equiv="refresh"]')).toBeNull();
    expect(
      document.querySelectorAll('meta[http-equiv="Content-Security-Policy"]'),
    ).toHaveLength(1);
    expect(
      document
        .querySelector('meta[data-textbooklens-csp]')
        ?.getAttribute('content'),
    ).toContain("connect-src 'none'");
    expect(document.querySelector('img')?.hasAttribute('src')).toBe(false);
    expect(document.querySelectorAll('img')[1]?.getAttribute('src')).toBe(
      'blob:http://localhost/synthetic-image',
    );
    expect(document.querySelector('a')?.hasAttribute('href')).toBe(false);
  });
});
