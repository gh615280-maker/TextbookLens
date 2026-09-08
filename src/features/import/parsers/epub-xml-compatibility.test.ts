import { createRequire } from 'node:module';

import {
  DOMImplementation,
  DOMParser as XmldomParser,
  XMLSerializer as XmldomSerializer,
} from '@xmldom/xmldom';
import { describe, expect, it } from 'vitest';

const require = createRequire(import.meta.url);
const { parse: parseWithEpubJs } = require('epubjs/lib/utils/core.js') as {
  parse(markup: string, mimeType: string, forceXmldom?: boolean): Document;
};
const xmldomPackage = require('@xmldom/xmldom/package.json') as {
  version: string;
};

const boundaryXml = `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE root>
<?fixture safe?>
<root><!--safe comment--><![CDATA[safe CDATA text]]><child>text</child></root>`;

describe('EPUB.js xmldom compatibility', () => {
  it('keeps the browser-native and forced fallback parser paths compatible', () => {
    expect(xmldomPackage.version).toBe('0.8.15');

    const nativeDocument = parseWithEpubJs(boundaryXml, 'application/xml');
    const fallbackDocument = parseWithEpubJs(
      boundaryXml,
      'application/xml',
      true,
    );

    for (const parsed of [nativeDocument, fallbackDocument]) {
      expect(parsed.documentElement.nodeName).toBe('root');
      expect(parsed.doctype?.name).toBe('root');
      expect(parsed.getElementsByTagName('child').item(0)?.textContent).toBe(
        'text',
      );
      expect(childNodeTypes(parsed)).toContain(
        Node.PROCESSING_INSTRUCTION_NODE,
      );
      expect(childNodeTypes(parsed.documentElement)).toEqual(
        expect.arrayContaining([Node.COMMENT_NODE, Node.CDATA_SECTION_NODE]),
      );
    }
  });

  it('serializes deeply nested fallback DOM trees without recursive stack failure', () => {
    const implementation = new DOMImplementation();
    const document = implementation.createDocument(null, 'root', null);
    let parent = document.documentElement;
    const depth = 12_000;
    for (let index = 0; index < depth; index += 1) {
      const child = document.createElement('n');
      parent.appendChild(child);
      parent = child;
    }
    parent.appendChild(document.createTextNode('bounded'));

    const serialized = strictSerialize(document);
    expect(serialized.startsWith('<root><n>')).toBe(true);
    expect(serialized.endsWith('</n></root>')).toBe(true);
    expect(serialized.length).toBe(depth * 7 + '<root>bounded</root>'.length);
  });

  it('rejects comment, PI, DocumentType, and CDATA injection boundaries', () => {
    const implementation = new DOMImplementation();

    const commentDocument = implementation.createDocument(null, 'root', null);
    const comment = commentDocument.createComment('safe');
    comment.data = '--><injected/><!--';
    commentDocument.documentElement.appendChild(comment);
    expect(() => strictSerialize(commentDocument)).toThrowError(
      /comment node data/iu,
    );

    const piDocument = implementation.createDocument(null, 'root', null);
    const instruction = piDocument.createProcessingInstruction(
      'fixture',
      'safe',
    );
    instruction.data = '?><injected/><?fixture ';
    piDocument.insertBefore(instruction, piDocument.documentElement);
    expect(() => strictSerialize(piDocument)).toThrowError(
      /ProcessingInstruction data/iu,
    );

    const doctype = implementation.createDocumentType('root', '', '"safe"');
    (doctype as DocumentType & { systemId: string }).systemId =
      '"safe"><injected/>';
    const doctypeDocument = implementation.createDocument(
      null,
      'root',
      doctype,
    );
    expect(() => strictSerialize(doctypeDocument)).toThrowError(
      /DocumentType systemId/iu,
    );

    const cdataDocument = implementation.createDocument(null, 'root', null);
    let cdataCreationError: unknown;
    try {
      cdataDocument.createCDATASection('safe]]><injected/>');
    } catch (error) {
      cdataCreationError = error;
    }
    expect(cdataCreationError).toMatchObject({
      code: 5,
      message: expect.stringContaining('data contains "]]>"'),
    });
    const cdata = cdataDocument.createCDATASection('safe');
    cdata.appendData(']]><injected/>');
    cdataDocument.documentElement.appendChild(cdata);
    expect(() => strictSerialize(cdataDocument)).toThrowError(
      /CDATASection data/iu,
    );
  });

  it('round-trips valid XML node boundaries without creating extra markup', () => {
    const document = new XmldomParser().parseFromString(
      boundaryXml.replace(/^<\?xml[^?]*\?>\s*/u, ''),
      'application/xml',
    );
    const serialized = strictSerialize(document);
    const reparsed = new XmldomParser().parseFromString(
      serialized,
      'application/xml',
    );

    expect(reparsed.getElementsByTagName('injected')).toHaveLength(0);
    expect(reparsed.doctype?.name).toBe('root');
    expect(childNodeTypes(reparsed)).toContain(
      Node.PROCESSING_INSTRUCTION_NODE,
    );
    expect(childNodeTypes(reparsed.documentElement)).toEqual(
      expect.arrayContaining([Node.COMMENT_NODE, Node.CDATA_SECTION_NODE]),
    );
  });

  it('rejects a reserved XML processing-instruction target in strict serialization', () => {
    // xmldom represents an XML declaration as a PI; strict DOM serialization
    // now correctly rejects the reserved target instead of treating it as an ordinary PI.
    const document = new XmldomParser().parseFromString(
      boundaryXml,
      'application/xml',
    );
    expect(() => strictSerialize(document)).toThrowError(
      /processing instruction target/iu,
    );
  });

  it('rejects invalid entity-reference names at creation and strict serialization', () => {
    const document = new DOMImplementation().createDocument(
      null,
      'root',
      null,
    ) as XMLDocument & { createEntityReference(name: string): Node };
    expect(() => document.createEntityReference('invalid<name')).toThrow();
    const reference = document.createEntityReference('valid');
    expect(strictSerialize(reference)).toBe('&valid;');
    (reference as Node & { nodeName: string }).nodeName = 'invalid<name';
    expect(() => strictSerialize(reference)).toThrow();
  });
});

function strictSerialize(node: Node): string {
  return new XmldomSerializer().serializeToString(node, false, undefined, {
    requireWellFormed: true,
  });
}

function childNodeTypes(node: Node): number[] {
  return Array.from(
    { length: node.childNodes.length },
    (_, index) => node.childNodes.item(index)?.nodeType ?? -1,
  );
}
