// Blob URLs change between openings. Keep archive-relative resource references
// private to each DOM element so persisted region hashes remain stable.
const references = new WeakMap<
  Element,
  ReadonlyMap<string, { archive: string; rendered: string }>
>();
const marker = 'data-textbooklens-resource-references';
const resourceAttributes = new Set([
  'src',
  'srcset',
  'href',
  'xlink:href',
  'poster',
]);

/** Called on the archive document before EPUB.js substitutes resource URLs. */
export function rememberEpubResourceReferences(document: Document): void {
  for (const element of document.querySelectorAll('*')) {
    // Never trust a marker supplied by the book itself.
    element.removeAttribute(marker);
    const entries = [...element.attributes]
      .filter(
        (attribute) =>
          resourceAttributes.has(attribute.name.toLowerCase()) &&
          isArchivePath(attribute.value),
      )
      .map(
        (attribute) => [attribute.name.toLowerCase(), attribute.value] as const,
      );
    if (entries.length === 0) continue;
    references.set(
      element,
      new Map(
        entries.map(([name, value]) => [
          name,
          { archive: value, rendered: value },
        ]),
      ),
    );
    // Encoding prevents the library's HTML string replacement from replacing
    // these original paths along with the visible resource attributes.
    element.setAttribute(
      marker,
      btoa(encodeURIComponent(JSON.stringify(entries))),
    );
  }
}

/** Called only on rendered documents produced by the preparation hook above. */
export function restoreEpubResourceReferences(document: Document): void {
  for (const element of document.querySelectorAll(`[${marker}]`)) {
    const encoded = element.getAttribute(marker)!;
    element.removeAttribute(marker);
    if (encoded.length > 65_536) continue;
    try {
      const entries: unknown = JSON.parse(decodeURIComponent(atob(encoded)));
      if (!Array.isArray(entries)) continue;
      const valid = entries.filter(
        (entry): entry is [string, string] =>
          Array.isArray(entry) &&
          entry.length === 2 &&
          typeof entry[0] === 'string' &&
          resourceAttributes.has(entry[0]) &&
          typeof entry[1] === 'string' &&
          isArchivePath(entry[1]),
      );
      references.set(
        element,
        new Map(
          valid.map(([name, value]) => [
            name,
            { archive: value, rendered: element.getAttribute(name) ?? '' },
          ]),
        ),
      );
    } catch {
      /* Invalid metadata cannot establish a stable resource identity. */
    }
  }
}

export function canonicalEpubAttribute(
  element: Element,
  attribute: Attr,
): string {
  const reference = references.get(element)?.get(attribute.name.toLowerCase());
  return reference?.rendered === attribute.value
    ? reference.archive
    : attribute.value;
}

function isArchivePath(value: string): boolean {
  return (
    value.length > 0 &&
    value.length <= 8_192 &&
    !/^(?:[a-z][a-z\d+.-]*:|\/\/|#)/iu.test(value.trim())
  );
}
