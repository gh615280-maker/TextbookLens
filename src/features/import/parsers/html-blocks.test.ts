import { describe, expect, it } from 'vitest';

import { collectHtmlBlocks, isEquationText } from './html-blocks';

describe('collectHtmlBlocks', () => {
  it('keeps recognized structural nodes in document order and excludes executable content', () => {
    const document = new DOMParser().parseFromString(
      `<body>
        <h1> Chapter   one </h1><p>First\n paragraph</p>
        <ul><li>One</li><li>Two</li></ul>
        <table><tr><th>x</th><th>y</th></tr><tr><td>0</td><td>1</td></tr></table>
        <figcaption>Figure caption</figcaption><math><mi>E</mi><mo>=</mo><mi>mc²</mi></math>
        <script>window.bad = true</script><style>p { color: red }</style><noscript>no</noscript>
      </body>`,
      'text/html',
    );

    expect(
      collectHtmlBlocks(document).map(({ kind, text }) => ({ kind, text })),
    ).toEqual([
      { kind: 'heading', text: 'Chapter one' },
      { kind: 'paragraph', text: 'First paragraph' },
      { kind: 'list', text: 'One' },
      { kind: 'list', text: 'Two' },
      { kind: 'table', text: 'x\ty\n0\t1' },
      { kind: 'caption', text: 'Figure caption' },
      { kind: 'equation', text: 'E=mc²' },
    ]);
  });

  it('requires mathematical operators to have surrounding symbols', () => {
    expect(isEquationText('E = mc²')).toBe(true);
    expect(isEquationText('y = 2x + 1')).toBe(true);
    expect(isEquationText('x - 1')).toBe(true);
    expect(isEquationText('well-known prose')).toBe(false);
    expect(isEquationText('separator -')).toBe(false);
  });
});
