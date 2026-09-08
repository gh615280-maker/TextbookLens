import { Buffer } from 'node:buffer';
import {
  link,
  mkdtemp,
  mkdir,
  realpath,
  rm,
  symlink,
  writeFile,
} from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';

import { afterEach, describe, expect, it } from 'vitest';

import {
  ArtifactAuditError,
  auditCargoManifestPins,
  auditArtifacts,
  parseArguments,
} from './audit-release-artifacts.mjs';
import { makeSentinel } from './check-sensitive-files.mjs';

const temporaryRoots = [];

afterEach(async () => {
  await Promise.all(
    temporaryRoots
      .splice(0)
      .map((root) => rm(root, { recursive: true, force: true })),
  );
});

async function temporaryRoot() {
  // Windows TEMP may use an 8.3 alias. Normal fixtures need canonical paths;
  // the tests below still pass their deliberately created aliases unchanged.
  const root = await mkdtemp(
    path.join(await realpath(tmpdir()), 'textbooklens-artifact-test-'),
  );
  temporaryRoots.push(root);
  return root;
}

async function expectAuditCode(action, code) {
  await expect(action()).rejects.toMatchObject({
    name: 'ArtifactAuditError',
    code,
  });
}

describe('release artifact path policy', () => {
  it('rejects relative, dot-segment, UNC, device, environment, glob, and broad roots', async () => {
    const invalid = [
      ['--file', 'relative.exe', 'RELATIVE_PATH'],
      [
        '--file',
        `${path.parse(process.cwd()).root}safe\\..\\artifact.exe`,
        'PATH_TRAVERSAL',
      ],
      [
        '--file',
        ['\\\\server', 'share', 'artifact.exe'].join('\\'),
        'UNSAFE_PATH_KIND',
      ],
      [
        '--file',
        ['\\\\?', 'C:', 'artifact.exe'].join('\\'),
        'UNSAFE_PATH_KIND',
      ],
      [
        '--file',
        `${path.parse(process.cwd()).root}%TEMP%\\artifact.exe`,
        'UNEXPANDED_PATH',
      ],
      [
        '--file',
        `${path.parse(process.cwd()).root}artifacts\\*.exe`,
        'UNEXPANDED_PATH',
      ],
      ['--root', path.parse(process.cwd()).root, 'BROAD_ROOT'],
    ];
    for (const [argument, value, code] of invalid) {
      const targets = parseArguments(['--no-defaults', argument, value]);
      await expectAuditCode(
        () => auditArtifacts(targets, { skipNetworkAudit: true }),
        code,
      );
    }
  });

  it('requires explicit labels for optional expected targets', () => {
    expect(() =>
      parseArguments(['--no-defaults', '--expect-file', 'missing-label']),
    ).toThrowError(
      expect.objectContaining({
        name: 'ArtifactAuditError',
        code: 'INVALID_ARGUMENT',
      }),
    );
    expect(() => parseArguments(['--no-defaults'])).toThrowError(
      expect.objectContaining({ code: 'NO_TARGETS' }),
    );
    expect(() =>
      parseArguments([
        '--no-defaults',
        '--expect-file',
        `unsafe\nlabel=${path.join(path.parse(process.cwd()).root, 'safe.bin')}`,
      ]),
    ).toThrowError(expect.objectContaining({ code: 'INVALID_ARGUMENT' }));
  });
});

describe('Cargo direct dependency policy', () => {
  it('accepts exact registry pins in normal and target dependency sections', () => {
    expect(
      auditCargoManifestPins(`
[dependencies]
serde = { version = "=1.0.0", features = ["derive"] }
[target.'cfg(windows)'.dependencies]
keyring = "=2.0.0"
`),
    ).toEqual([]);
  });

  it('rejects ranges, missing versions, and git/path/workspace sources', () => {
    const issues = auditCargoManifestPins(`
[dependencies]
ranged = "1.2.3"
from_git = { git = "https://example.invalid/repository" }
from_path = { version = "=1.0.0", path = "../crate" }
workspace_dep = { workspace = true }
custom_registry = { version = "=1.0.0", registry = "private" }
[dependencies.table_style]
version = "=1.0.0"
`);
    expect(issues).toEqual(
      expect.arrayContaining([
        expect.stringContaining(
          'ranged: direct Cargo dependency must use an exact',
        ),
        expect.stringContaining(
          'from_git: direct Cargo dependency must come from',
        ),
        expect.stringContaining(
          'from_path: direct Cargo dependency must come from',
        ),
        expect.stringContaining(
          'workspace_dep: direct Cargo dependency must come from',
        ),
        expect.stringContaining(
          'custom_registry: direct Cargo dependency must come from',
        ),
        expect.stringContaining(
          'per-dependency Cargo tables are not permitted',
        ),
      ]),
    );
  });
});

describe('release artifact scanning', () => {
  it('scans explicit roots/files, reports optional absence, and never executes artifacts', async () => {
    const root = await temporaryRoot();
    const artifacts = path.join(root, 'artifacts');
    await mkdir(artifacts);
    const executionMarker = path.join(root, 'must-not-exist.txt');
    await writeFile(
      path.join(artifacts, 'payload.js'),
      `require('node:fs').writeFileSync(${JSON.stringify(executionMarker)}, 'bad')`,
    );
    const optional = path.join(root, 'missing', 'installer.exe');

    const result = await auditArtifacts(
      [
        { label: 'artifacts', kind: 'root', path: artifacts },
        { label: 'installer', kind: 'file', path: optional, optional: true },
      ],
      { skipNetworkAudit: true },
    );
    expect(result.findings).toEqual([]);
    expect(result.reports).toEqual(
      expect.arrayContaining([
        expect.objectContaining({ label: 'artifacts', status: 'SCANNED' }),
        expect.objectContaining({ label: 'installer', status: 'NOT PRESENT' }),
      ]),
    );
    await expect(
      import('node:fs/promises').then(({ lstat }) => lstat(executionMarker)),
    ).rejects.toMatchObject({ code: 'ENOENT' });
  });

  it('does not report PASS when every explicit target is absent', async () => {
    const root = await temporaryRoot();
    const missing = path.join(root, 'missing', 'installer.exe');
    await expect(
      auditArtifacts(
        [{ label: 'installer', kind: 'file', path: missing, optional: true }],
        { skipNetworkAudit: true },
      ),
    ).rejects.toMatchObject({ code: 'NO_PRESENT_TARGETS' });
  });

  it('detects sentinel content in binary artifacts', async () => {
    const root = await temporaryRoot();
    const artifact = path.join(root, 'debug.exe');
    await writeFile(
      artifact,
      Buffer.concat([
        Buffer.from([0, 1, 0, 2]),
        Buffer.from(makeSentinel('credential_identifier'), 'utf16le'),
      ]),
    );

    const result = await auditArtifacts(
      [{ label: 'debug', kind: 'file', path: artifact }],
      { skipNetworkAudit: true },
    );
    expect(result.findings.map((finding) => finding.rule)).toContain(
      'sentinel:credential_identifier',
    );
  });

  it('rejects duplicate paths and hard-linked identities', async () => {
    const root = await temporaryRoot();
    const first = path.join(root, 'first.bin');
    const second = path.join(root, 'second.bin');
    await writeFile(first, 'safe');

    await expect(
      auditArtifacts(
        [
          { label: 'first', kind: 'file', path: first },
          { label: 'duplicate', kind: 'file', path: first },
        ],
        { skipNetworkAudit: true },
      ),
    ).rejects.toMatchObject({ code: 'DUPLICATE_TARGET' });

    await link(first, second);
    await expect(
      auditArtifacts([{ label: 'first', kind: 'file', path: first }], {
        skipNetworkAudit: true,
      }),
    ).rejects.toMatchObject({ code: 'HARDLINK' });
  });

  it('rejects junctions or symlinks when the platform permits creating them', async () => {
    const root = await temporaryRoot();
    const target = path.join(root, 'target');
    const alias = path.join(root, 'alias');
    await mkdir(target);
    await writeFile(path.join(target, 'safe.bin'), 'safe');
    try {
      await symlink(
        target,
        alias,
        process.platform === 'win32' ? 'junction' : 'dir',
      );
    } catch (error) {
      if (error?.code === 'EPERM') return;
      throw error;
    }
    await expect(
      auditArtifacts([{ label: 'alias', kind: 'root', path: alias }], {
        skipNetworkAudit: true,
      }),
    ).rejects.toMatchObject({ code: 'REPARSE_POINT' });
  });

  it('detects mutation races and enforces file/count/depth limits', async () => {
    const root = await temporaryRoot();
    const artifact = path.join(root, 'race.bin');
    await writeFile(artifact, 'first');
    await expect(
      auditArtifacts([{ label: 'race', kind: 'file', path: artifact }], {
        skipNetworkAudit: true,
        afterOpen: async ({ absolutePath }) => {
          await writeFile(absolutePath, 'changed-during-read');
        },
      }),
    ).rejects.toMatchObject({ code: 'RACE_DETECTED' });

    const limitedRoot = path.join(root, 'limited');
    await mkdir(limitedRoot);
    await writeFile(path.join(limitedRoot, 'one.bin'), 'one');
    await writeFile(path.join(limitedRoot, 'two.bin'), 'two');
    await expect(
      auditArtifacts([{ label: 'limited', kind: 'root', path: limitedRoot }], {
        skipNetworkAudit: true,
        limits: { maxFiles: 1 },
      }),
    ).rejects.toMatchObject({ code: 'FILE_COUNT_LIMIT' });
  });

  it('uses stable typed safety errors', () => {
    const error = new ArtifactAuditError('SAFE_CODE', 'safe message');
    expect(error).toMatchObject({
      name: 'ArtifactAuditError',
      code: 'SAFE_CODE',
    });
  });
});
