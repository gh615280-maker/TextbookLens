import { describe, expect, it } from 'vitest';

import {
  auditManifestAndLockfile,
  isLicenseExpressionAllowed,
  normalizeLicenseExpression,
  parseSpdxExpression,
} from './check-npm-licenses.mjs';

const allowed = new Set(['Apache-2.0', 'MIT', 'BSD-2-Clause']);

describe('npm license expressions', () => {
  it('accepts SPDX OR when at least one branch is allowed', () => {
    expect(
      isLicenseExpressionAllowed('(MIT OR GPL-3.0-or-later)', allowed),
    ).toBe(true);
    expect(isLicenseExpressionAllowed('(MPL-2.0 OR Apache-2.0)', allowed)).toBe(
      true,
    );
  });

  it('requires every SPDX AND branch and preserves precedence', () => {
    expect(
      isLicenseExpressionAllowed('MIT AND GPL-3.0-or-later', allowed),
    ).toBe(false);
    expect(
      isLicenseExpressionAllowed(
        'MIT OR (Apache-2.0 AND GPL-3.0-or-later)',
        allowed,
      ),
    ).toBe(true);
    expect(
      isLicenseExpressionAllowed(
        'GPL-3.0-or-later OR (Apache-2.0 AND MPL-2.0)',
        allowed,
      ),
    ).toBe(false);
  });

  it('requires an explicit package-level allowance for WITH', () => {
    const expression = 'Apache-2.0 WITH LLVM-exception';
    expect(isLicenseExpressionAllowed(expression, allowed)).toBe(false);
    expect(
      isLicenseExpressionAllowed(
        expression,
        allowed,
        new Set(['LLVM-exception']),
      ),
    ).toBe(true);
  });

  it('rejects invalid expressions', () => {
    expect(() => parseSpdxExpression('BSD*')).toThrow();
    expect(() => parseSpdxExpression('MIT OR')).toThrow();
  });

  it('keeps duck BSD metadata normalization exact and version-scoped', () => {
    expect(normalizeLicenseExpression('duck@0.1.12', 'BSD*')).toBe(
      'BSD-2-Clause',
    );
    expect(normalizeLicenseExpression('duck@0.1.13', 'BSD*')).toBe('BSD*');
    expect(normalizeLicenseExpression('other@1.0.0', 'BSD*')).toBe('BSD*');
  });
});

function supplyChainFixture() {
  return {
    manifest: {
      name: 'fixture-app',
      version: '1.0.0',
      dependencies: { runtime: '2.3.4' },
      devDependencies: { tooling: '5.6.7' },
    },
    lockfile: {
      name: 'fixture-app',
      version: '1.0.0',
      lockfileVersion: 3,
      packages: {
        '': {
          name: 'fixture-app',
          version: '1.0.0',
          dependencies: { runtime: '2.3.4' },
          devDependencies: { tooling: '5.6.7' },
        },
        'node_modules/runtime': {
          version: '2.3.4',
          resolved: 'https://registry.npmjs.org/runtime/-/runtime-2.3.4.tgz',
          integrity: `sha512-${'A'.repeat(86)}==`,
        },
        'node_modules/tooling': {
          version: '5.6.7',
          resolved: 'https://registry.npmjs.org/tooling/-/tooling-5.6.7.tgz',
          integrity: `sha512-${'B'.repeat(86)}==`,
        },
      },
    },
  };
}

describe('npm manifest and lockfile supply-chain policy', () => {
  it('accepts exact direct pins and npm HTTPS records with SHA-512 integrity', () => {
    const { manifest, lockfile } = supplyChainFixture();
    expect(auditManifestAndLockfile(manifest, lockfile)).toEqual([]);
  });

  it('rejects ranges, root drift, and direct package version drift', () => {
    const { manifest, lockfile } = supplyChainFixture();
    manifest.dependencies.runtime = '^2.3.4';
    lockfile.packages[''].devDependencies.tooling = '5.6.8';
    lockfile.packages['node_modules/runtime'].version = '2.3.5';

    expect(auditManifestAndLockfile(manifest, lockfile)).toEqual(
      expect.arrayContaining([
        expect.stringContaining('direct dependency must use an exact semver'),
        expect.stringContaining('root declaration drift'),
        expect.stringContaining('exact direct version drift'),
      ]),
    );
  });

  it('rejects non-registry sources and absent or weak integrity metadata', () => {
    const { manifest, lockfile } = supplyChainFixture();
    lockfile.packages['node_modules/runtime'].resolved =
      'https://packages.invalid/runtime.tgz';
    lockfile.packages['node_modules/tooling'].integrity = 'sha1-weak';

    expect(auditManifestAndLockfile(manifest, lockfile)).toEqual(
      expect.arrayContaining([
        expect.stringContaining('source is not the npm HTTPS registry'),
        expect.stringContaining('missing SHA-512 lockfile integrity'),
      ]),
    );
  });

  it('rejects undeclared locked roots and unsupported lockfile versions', () => {
    const { manifest, lockfile } = supplyChainFixture();
    lockfile.lockfileVersion = 2;
    lockfile.packages[''].dependencies.ghost = '9.9.9';

    expect(auditManifestAndLockfile(manifest, lockfile)).toEqual(
      expect.arrayContaining([
        expect.stringContaining('lockfileVersion must be 3'),
        expect.stringContaining('undeclared root dependency'),
      ]),
    );
  });
});
