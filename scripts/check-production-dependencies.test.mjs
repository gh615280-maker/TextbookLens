import { describe, expect, it } from 'vitest';

import {
  auditProductionDependencyPolicy,
  isAffectedXmldomVersion,
  productionXmldomVersions,
} from './check-production-dependencies.mjs';

function fixture() {
  return {
    manifest: {
      dependencies: { epubjs: '0.3.93' },
      overrides: {
        'epubjs@0.3.93': { '@xmldom/xmldom': '0.8.13' },
      },
    },
    lockfile: {
      lockfileVersion: 3,
      packages: {
        '': { dependencies: { epubjs: '0.3.93' } },
        'node_modules/epubjs': {
          version: '0.3.93',
          dependencies: { '@xmldom/xmldom': '^0.7.5' },
        },
        'node_modules/@xmldom/xmldom': { version: '0.8.13' },
      },
    },
  };
}

describe('production dependency security policy', () => {
  it('accepts the exact patched epubjs override and reports its production version', () => {
    const { manifest, lockfile } = fixture();
    expect(auditProductionDependencyPolicy(manifest, lockfile)).toEqual([]);
    expect(productionXmldomVersions(lockfile)).toEqual(['0.8.13']);
  });

  it('rejects every reviewed vulnerable @xmldom/xmldom release line', () => {
    expect(isAffectedXmldomVersion('0.7.13')).toBe(true);
    expect(isAffectedXmldomVersion('0.8.12')).toBe(true);
    expect(isAffectedXmldomVersion('0.8.13')).toBe(false);
    expect(isAffectedXmldomVersion('0.9.9')).toBe(true);
    expect(isAffectedXmldomVersion('0.9.10')).toBe(false);

    const { manifest, lockfile } = fixture();
    lockfile.packages['node_modules/@xmldom/xmldom'].version = '0.7.13';
    expect(auditProductionDependencyPolicy(manifest, lockfile)).toEqual(
      expect.arrayContaining([
        expect.stringContaining('@xmldom/xmldom 0.7.13 is affected'),
        expect.stringContaining('does not contain the epubjs override 0.8.13'),
      ]),
    );
  });

  it('rejects the unpatched package name but ignores a dev-only legacy copy', () => {
    const { manifest, lockfile } = fixture();
    lockfile.packages['node_modules/xmldom'] = { version: '0.6.0' };
    lockfile.packages['node_modules/tool/node_modules/@xmldom/xmldom'] = {
      version: '0.7.13',
      dev: true,
    };

    expect(auditProductionDependencyPolicy(manifest, lockfile)).toEqual([
      expect.stringContaining('unscoped xmldom has no patched release'),
    ]);
  });

  it('requires a scoped exact safe override when the epubjs range is unsafe', () => {
    const { manifest, lockfile } = fixture();
    manifest.overrides = {
      epubjs: { '@xmldom/xmldom': '^0.8.13' },
    };

    expect(auditProductionDependencyPolicy(manifest, lockfile)).toEqual(
      expect.arrayContaining([
        expect.stringContaining('override must use an exact release semver'),
        expect.stringContaining(
          'overrides.epubjs@0.3.93.@xmldom/xmldom must pin an exact patched release',
        ),
      ]),
    );
  });
});
