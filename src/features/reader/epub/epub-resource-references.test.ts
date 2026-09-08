import { expect, it } from 'vitest';
import { hashEpubRegionElement } from './epub-region-capture';
import {
  rememberEpubResourceReferences,
  restoreEpubResourceReferences,
} from './epub-resource-references';

it('keeps a region hash stable across blob URL replacement and reopening', async () => {
  const source = new DOMParser().parseFromString(
    '<body><figure><img src="../Images/plot.png" alt="Synthetic plot"/><figcaption>A local diagram</figcaption></figure></body>',
    'text/html',
  );
  rememberEpubResourceReferences(source);
  const expected = await hashEpubRegionElement(source.querySelector('figure')!);
  for (const url of ['blob:synthetic-first', 'blob:synthetic-reopened']) {
    const rendered = new DOMParser().parseFromString(
      source.documentElement.outerHTML.replace(
        'src="../Images/plot.png"',
        `src="${url}"`,
      ),
      'text/html',
    );
    restoreEpubResourceReferences(rendered);
    expect(rendered.querySelector('img')!.getAttribute('src')).toBe(url);
    expect(await hashEpubRegionElement(rendered.querySelector('figure')!)).toBe(
      expected,
    );
    expect(
      rendered.querySelector('[data-textbooklens-resource-references]'),
    ).toBeNull();
    rendered.querySelector('img')!.setAttribute('src', 'blob:changed-image');
    expect(
      await hashEpubRegionElement(rendered.querySelector('figure')!),
    ).not.toBe(expected);
  }
  source.querySelector('img')!.setAttribute('src', '../Images/different.png');
  rememberEpubResourceReferences(source);
  expect(await hashEpubRegionElement(source.querySelector('figure')!)).not.toBe(
    expected,
  );
});

it('overwrites untrusted book markers and safely ignores malformed rendered markers', async () => {
  const document = new DOMParser().parseFromString(
    '<body><img src="actual.png" data-textbooklens-resource-references="fake"/></body>',
    'text/html',
  );
  rememberEpubResourceReferences(document);
  expect(
    document
      .querySelector('img')!
      .getAttribute('data-textbooklens-resource-references'),
  ).not.toBe('fake');
  document
    .querySelector('img')!
    .setAttribute('data-textbooklens-resource-references', 'not-valid');
  expect(() => restoreEpubResourceReferences(document)).not.toThrow();
});
