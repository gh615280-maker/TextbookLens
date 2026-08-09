import { describe, expect, it } from 'vitest';

import {
  auditDependencyPolicy,
  dependencyVersionsByEnvironment,
  isAffectedDompurifyVersion,
  isAffectedNanoidVersion,
  isAffectedXmldomVersion,
  productionXmldomVersions,
} from './check-production-dependencies.mjs';

function fixture() {
  return {
    manifest: {
      dependencies: { dompurify: '3.4.13', epubjs: '0.3.93' },
      overrides: {
        'epubjs@0.3.93': { '@xmldom/xmldom': '0.8.13' },
      },
    },
    lockfile: {
      lockfileVersion: 3,
      packages: {
        '': {
          dependencies: { dompurify: '3.4.13', epubjs: '0.3.93' },
        },
        'node_modules/dompurify': { version: '3.4.13' },
        'node_modules/epubjs': {
          version: '0.3.93',
          dependencies: { '@xmldom/xmldom': '^0.7.5' },
        },
        'node_modules/@xmldom/xmldom': { version: '0.8.13' },
        'node_modules/postcss': {
          version: '8.5.26',
          dev: true,
          dependencies: { nanoid: '^3.3.17' },
        },
        'node_modules/nanoid': { version: '3.3.17', dev: true },
        'node_modules/docx/node_modules/nanoid': {
          version: '5.1.16',
          dev: true,
        },
      },
    },
  };
}

describe('supply-chain dependency security policy', () => {
  it('accepts patched production and development graphs and reports them separately', () => {
    const { manifest, lockfile } = fixture();
    expect(auditDependencyPolicy(manifest, lockfile)).toEqual([]);
    expect(dependencyVersionsByEnvironment(lockfile, 'dompurify')).toEqual({
      production: ['3.4.13'],
      development: [],
    });
    expect(dependencyVersionsByEnvironment(lockfile, 'nanoid')).toEqual({
      production: [],
      development: ['3.3.17', '5.1.16'],
    });
    expect(productionXmldomVersions(lockfile)).toEqual(['0.8.13']);
  });

  it('rejects affected DOMPurify releases and requires an exact direct patch', () => {
    expect(isAffectedDompurifyVersion('3.4.12')).toBe(true);
    expect(isAffectedDompurifyVersion('3.4.13')).toBe(false);
    expect(isAffectedDompurifyVersion('4.0.0')).toBe(false);
    expect(isAffectedDompurifyVersion('not-semver')).toBe(true);

    const { manifest, lockfile } = fixture();
    manifest.dependencies.dompurify = '^3.4.13';
    lockfile.packages['node_modules/dompurify'].version = '3.4.12';
    expect(auditDependencyPolicy(manifest, lockfile)).toEqual(
      expect.arrayContaining([
        expect.stringContaining('must pin an exact release semver'),
        expect.stringContaining(
          '[production]: DOMPurify 3.4.12 is affected by GHSA-55q2-fjhq-7xh7',
        ),
      ]),
    );
  });

  it('rejects every affected nanoid release line in the full graph', () => {
    expect(isAffectedNanoidVersion('3.3.16')).toBe(true);
    expect(isAffectedNanoidVersion('3.3.17')).toBe(false);
    expect(isAffectedNanoidVersion('3.3.18')).toBe(false);
    expect(isAffectedNanoidVersion('4.0.0')).toBe(true);
    expect(isAffectedNanoidVersion('5.1.5')).toBe(true);
    expect(isAffectedNanoidVersion('5.1.6')).toBe(false);
    expect(isAffectedNanoidVersion('6.0.0')).toBe(false);
    expect(isAffectedNanoidVersion('not-semver')).toBe(true);

    const { manifest, lockfile } = fixture();
    lockfile.packages['node_modules/nanoid'].version = '3.3.16';
    lockfile.packages['node_modules/docx/node_modules/nanoid'].version =
      '5.1.5';
    lockfile.packages['node_modules/tool/node_modules/nanoid'] = { dev: true };
    expect(auditDependencyPolicy(manifest, lockfile)).toEqual(
      expect.arrayContaining([
        expect.stringContaining(
          '[development]: nanoid 3.3.16 is affected by GHSA-2v37-7h3g-55p8',
        ),
        expect.stringContaining(
          '[development]: nanoid 5.1.5 is affected by GHSA-2v37-7h3g-55p8',
        ),
        expect.stringContaining(
          '[development]: missing exact nanoid version; GHSA-2v37-7h3g-55p8 is fail-closed',
        ),
      ]),
    );
  });

  it('rejects every reviewed vulnerable @xmldom/xmldom release line', () => {
    expect(isAffectedXmldomVersion('0.7.13')).toBe(true);
    expect(isAffectedXmldomVersion('0.8.12')).toBe(true);
    expect(isAffectedXmldomVersion('0.8.13')).toBe(false);
    expect(isAffectedXmldomVersion('0.9.9')).toBe(true);
    expect(isAffectedXmldomVersion('0.9.10')).toBe(false);

    const { manifest, lockfile } = fixture();
    lockfile.packages['node_modules/@xmldom/xmldom'].version = '0.7.13';
    expect(auditDependencyPolicy(manifest, lockfile)).toEqual(
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

    expect(auditDependencyPolicy(manifest, lockfile)).toEqual([
      expect.stringContaining('unscoped xmldom has no patched release'),
    ]);
  });

  it('requires a scoped exact safe override when the epubjs range is unsafe', () => {
    const { manifest, lockfile } = fixture();
    manifest.overrides = {
      epubjs: { '@xmldom/xmldom': '^0.8.13' },
    };

    expect(auditDependencyPolicy(manifest, lockfile)).toEqual(
      expect.arrayContaining([
        expect.stringContaining('override must use an exact release semver'),
        expect.stringContaining(
          'overrides.epubjs@0.3.93.@xmldom/xmldom must pin an exact patched release',
        ),
      ]),
    );
  });
});
