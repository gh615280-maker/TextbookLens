import { Buffer } from 'node:buffer';
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';

import JSZip from 'jszip';
import { afterEach, describe, expect, it } from 'vitest';

import {
  SENTINEL_CLASSES,
  makeSentinel,
  scanBuffer,
  scanFile,
} from './check-sensitive-files.mjs';

const temporaryRoots = [];

afterEach(async () => {
  await Promise.all(
    temporaryRoots
      .splice(0)
      .map((root) => rm(root, { recursive: true, force: true })),
  );
});

async function temporaryRoot() {
  const root = await mkdtemp(
    path.join(tmpdir(), 'textbooklens-sensitive-test-'),
  );
  temporaryRoots.push(root);
  return root;
}

function rules(buffer) {
  return scanBuffer(buffer).map((issue) => issue.rule);
}

function fullWidth(value) {
  return [...value]
    .map((character) => {
      const code = character.codePointAt(0);
      if (code >= 0x21 && code <= 0x7e) {
        return String.fromCodePoint(code + 0xfee0);
      }
      return character;
    })
    .join('');
}

function utf16Be(value) {
  const littleEndian = Buffer.from(value, 'utf16le');
  for (let index = 0; index < littleEndian.length; index += 2) {
    [littleEndian[index], littleEndian[index + 1]] = [
      littleEndian[index + 1],
      littleEndian[index],
    ];
  }
  return littleEndian;
}

describe('sensitive sentinel matrix', () => {
  it('detects every maintained class without treating ordinary words as secrets', () => {
    expect(
      rules(
        Buffer.from(
          'credential identifier source path textbook image prompt instruction answer vendor response remote resource note request profile run attempt',
        ),
      ),
    ).toEqual([]);

    for (const sentinelClass of SENTINEL_CLASSES) {
      expect(rules(Buffer.from(makeSentinel(sentinelClass)))).toContain(
        `sentinel:${sentinelClass}`,
      );
    }
  });

  it('normalizes case, compatibility Unicode, JSON escapes, URL encoding, base64, and hex', () => {
    const sentinel = makeSentinel('prompt');
    const encodings = [
      sentinel.toLocaleLowerCase('en-US'),
      fullWidth(sentinel),
      JSON.stringify({ value: sentinel }).replaceAll('T', '\\u0054'),
      encodeURIComponent(sentinel),
      Buffer.from(sentinel).toString('base64'),
      Buffer.from(sentinel).toString('hex'),
    ];
    for (const encoded of encodings) {
      expect(rules(Buffer.from(encoded))).toContain('sentinel:prompt');
    }
  });

  it('detects values crossing read-buffer boundaries', async () => {
    const root = await temporaryRoot();
    const target = path.join(root, 'split.bin');
    await writeFile(
      target,
      Buffer.concat([
        Buffer.from('a'.repeat(31)),
        Buffer.from(makeSentinel('vendor_body')),
        Buffer.from('z'.repeat(31)),
      ]),
    );

    const findings = await scanFile(target, 'split.bin', {
      chunkBytes: 17,
      overlapBytes: 256,
    });
    expect(findings.map((issue) => issue.rule)).toContain(
      'sentinel:vendor_body',
    );
  });

  it('scans binary string tables plus UTF-8, UTF-16LE, and UTF-16BE', () => {
    const values = [
      ['textbook_text', Buffer.from(makeSentinel('textbook_text'), 'utf8')],
      [
        'internal_request_id',
        Buffer.from(makeSentinel('internal_request_id'), 'utf16le'),
      ],
      ['internal_attempt_id', utf16Be(makeSentinel('internal_attempt_id'))],
    ];
    for (const [sentinelClass, encoded] of values) {
      const binary = Buffer.concat([
        Buffer.from([0, 1, 2, 0]),
        encoded,
        Buffer.from([0, 255, 0]),
      ]);
      expect(rules(binary)).toContain(`sentinel:${sentinelClass}`);
    }
  });

  it('scans minified bundles and source-map sourcesContent', () => {
    const bundle = Buffer.from(
      `(()=>{const x=${JSON.stringify(
        Buffer.from(makeSentinel('answer')).toString('base64'),
      )}})();//# sourceMappingURL=app.js.map`,
    );
    const sourceMap = Buffer.from(
      JSON.stringify({
        version: 3,
        sources: ['app.ts'],
        sourcesContent: [makeSentinel('teaching_instruction')],
        mappings: '',
      }),
    );
    expect(rules(bundle)).toContain('sentinel:answer');
    expect(rules(sourceMap)).toContain('sentinel:teaching_instruction');
  });

  it('scans archive entry names, contents, and a nested archive', async () => {
    const root = await temporaryRoot();
    const inner = new JSZip();
    inner.file('safe.txt', makeSentinel('remote_resource_id'));
    const innerBytes = await inner.generateAsync({ type: 'nodebuffer' });

    const outer = new JSZip();
    outer.file(`${makeSentinel('user_note')}.txt`, 'ordinary fixture');
    outer.file('content.txt', makeSentinel('page_image_base64'));
    outer.file('nested.zip', innerBytes);
    const archivePath = path.join(root, 'fixture.zip');
    await writeFile(
      archivePath,
      await outer.generateAsync({ type: 'nodebuffer' }),
    );

    const findings = await scanFile(archivePath, 'fixture.zip');
    const archiveRules = findings.map((issue) => issue.rule);
    expect(archiveRules).toContain('sentinel:user_note');
    expect(archiveRules).toContain('sentinel:page_image_base64');
    expect(archiveRules).toContain('sentinel:remote_resource_id');
  });

  it('stops ZIP expansion at the configured entry and total byte limits', async () => {
    const root = await temporaryRoot();
    const archive = new JSZip();
    archive.file('large.txt', 'x'.repeat(4_096));
    const archivePath = path.join(root, 'bounded.zip');
    await writeFile(
      archivePath,
      await archive.generateAsync({
        type: 'nodebuffer',
        compression: 'DEFLATE',
      }),
    );

    const entryLimited = await scanFile(archivePath, 'bounded.zip', {
      maxArchiveEntryBytes: 128,
      maxArchiveExpandedBytes: 8_192,
    });
    expect(entryLimited.map((issue) => issue.rule)).toContain(
      'archive-entry-size-limit',
    );
    const totalLimited = await scanFile(archivePath, 'bounded.zip', {
      maxArchiveEntryBytes: 8_192,
      maxArchiveExpandedBytes: 128,
    });
    expect(totalLimited.map((issue) => issue.rule)).toContain(
      'archive-expanded-size-limit',
    );
  });

  it('detects credential shapes and private absolute paths without matching placeholders', () => {
    const positives = [
      ['sk', 'syntheticcredential123456'].join('-'),
      ['AIza', 'SyntheticCredential123456'].join(''),
      ['OPENAI_API_KEY', 'synthetic-value-123456'].join('='),
      ['Authorization:', 'Bearer', 'synthetic-value-123456'].join(' '),
      ['textbooklens', '10000000-0000-4000-8000-000000000001'].join('/'),
      ['C:', 'Users', 'PrivateUser', 'source.pdf'].join('\\'),
    ];
    for (const positive of positives) {
      expect(rules(Buffer.from(positive)).length).toBeGreaterThan(0);
    }
    expect(
      rules(
        Buffer.from(
          'sk-example API_KEY=<set-at-runtime> provider_profile_id=local-id C:\\fixtures\\source.pdf',
        ),
      ),
    ).toEqual([]);
  });
});
