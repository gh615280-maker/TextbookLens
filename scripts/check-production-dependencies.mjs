import { readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const moduleFile = import.meta.url.startsWith('file:')
  ? fileURLToPath(import.meta.url)
  : path.join(process.cwd(), 'scripts', 'check-production-dependencies.mjs');
const projectRoot = path.resolve(path.dirname(moduleFile), '..');

export const XMLDOM_ADVISORIES = Object.freeze([
  'GHSA-6gmq-8vp8-gcm6',
  'GHSA-wh4c-j3r5-mjhp',
  'GHSA-j759-j44w-7fr8',
  'GHSA-x6wf-f3px-wcqx',
  'GHSA-f6ww-3ggp-fr8h',
  'GHSA-2v35-w6hq-6mfw',
]);
export const DOMPURIFY_ADVISORIES = Object.freeze(['GHSA-55q2-fjhq-7xh7']);
export const NANOID_ADVISORIES = Object.freeze(['GHSA-2v37-7h3g-55p8']);

const EXACT_RELEASE = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/u;
const SIMPLE_SAFE_RANGE = /^(\^|~)?(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/u;

if (process.argv[1] && path.resolve(process.argv[1]) === moduleFile) {
  const manifest = JSON.parse(
    readFileSync(path.join(projectRoot, 'package.json'), 'utf8'),
  );
  const lockfile = JSON.parse(
    readFileSync(path.join(projectRoot, 'package-lock.json'), 'utf8'),
  );
  const issues = auditDependencyPolicy(manifest, lockfile);
  if (issues.length > 0) {
    for (const issue of issues) console.error(issue);
    process.exitCode = 1;
  } else {
    const dompurify = dependencyVersionsByEnvironment(lockfile, 'dompurify');
    const nanoid = dependencyVersionsByEnvironment(lockfile, 'nanoid');
    const xmldom = productionXmldomVersions(lockfile);
    const advisoryCount =
      XMLDOM_ADVISORIES.length +
      DOMPURIFY_ADVISORIES.length +
      NANOID_ADVISORIES.length;
    console.log(
      `Supply-chain dependency security policy passed: production DOMPurify ${formatVersions(dompurify.production)}; production nanoid ${formatVersions(nanoid.production)}; development nanoid ${formatVersions(nanoid.development)}; production @xmldom/xmldom ${formatVersions(xmldom)}; ${advisoryCount} reviewed advisories enforced across production and development dependency graphs.`,
    );
  }
}

export function auditDependencyPolicy(manifest, lockfile) {
  const issues = [];
  if (lockfile.lockfileVersion !== 3) {
    issues.push(
      'package-lock.json: security policy requires lockfileVersion 3',
    );
  }

  const dompurifySpecification = manifest.dependencies?.dompurify;
  if (
    typeof dompurifySpecification !== 'string' ||
    !EXACT_RELEASE.test(dompurifySpecification)
  ) {
    issues.push(
      'package.json:dependencies.dompurify must pin an exact release semver',
    );
  } else if (isAffectedDompurifyVersion(dompurifySpecification)) {
    issues.push(
      `package.json:dependencies.dompurify ${dompurifySpecification} is affected by ${DOMPURIFY_ADVISORIES.join(', ')}`,
    );
  }

  const lockedDompurify = lockfile.packages?.['node_modules/dompurify'];
  if (!lockedDompurify) {
    issues.push(
      'package-lock.json: production graph is missing direct DOMPurify',
    );
  } else {
    if (lockedDompurify.dev === true) {
      issues.push(
        'package-lock.json: direct DOMPurify must be classified as production',
      );
    }
    if (
      typeof dompurifySpecification === 'string' &&
      EXACT_RELEASE.test(dompurifySpecification) &&
      lockedDompurify.version !== dompurifySpecification
    ) {
      issues.push(
        `package-lock.json: direct DOMPurify ${lockedDompurify.version ?? 'missing'} does not match package.json ${dompurifySpecification}`,
      );
    }
  }

  for (const [packagePath, metadata] of Object.entries(
    lockfile.packages ?? {},
  )) {
    if (packagePath === '') continue;
    const packageName = packageNameFromLockPath(packagePath);
    const environment = metadata?.dev === true ? 'development' : 'production';
    if (packageName === 'dompurify') {
      if (typeof metadata.version !== 'string') {
        issues.push(
          `${packagePath} [${environment}]: missing exact DOMPurify version; ${DOMPURIFY_ADVISORIES.join(', ')} is fail-closed`,
        );
      } else if (isAffectedDompurifyVersion(metadata.version)) {
        issues.push(
          `${packagePath} [${environment}]: DOMPurify ${metadata.version} is affected by ${DOMPURIFY_ADVISORIES.join(', ')}`,
        );
      }
    }
    if (packageName === 'nanoid') {
      if (typeof metadata.version !== 'string') {
        issues.push(
          `${packagePath} [${environment}]: missing exact nanoid version; ${NANOID_ADVISORIES.join(', ')} is fail-closed`,
        );
      } else if (isAffectedNanoidVersion(metadata.version)) {
        issues.push(
          `${packagePath} [${environment}]: nanoid ${metadata.version} is affected by ${NANOID_ADVISORIES.join(', ')}`,
        );
      }
    }
  }

  const productionVersions = [];
  for (const [packagePath, metadata] of Object.entries(
    lockfile.packages ?? {},
  )) {
    if (packagePath === '' || metadata?.dev === true) continue;
    const packageName = packageNameFromLockPath(packagePath);
    if (packageName === 'xmldom') {
      issues.push(
        `${packagePath}: unscoped xmldom has no patched release for the reviewed advisory set`,
      );
      continue;
    }
    if (packageName !== '@xmldom/xmldom') continue;
    if (typeof metadata.version !== 'string') {
      issues.push(`${packagePath}: missing exact @xmldom/xmldom version`);
      continue;
    }
    productionVersions.push(metadata.version);
    if (isAffectedXmldomVersion(metadata.version)) {
      issues.push(
        `${packagePath}: @xmldom/xmldom ${metadata.version} is affected by ${XMLDOM_ADVISORIES.join(', ')}`,
      );
    }
  }

  for (const override of collectXmldomOverrides(manifest.overrides)) {
    if (!EXACT_RELEASE.test(override.specification)) {
      issues.push(
        `package.json:${override.path}: xmldom override must use an exact release semver`,
      );
    } else if (isAffectedXmldomVersion(override.specification)) {
      issues.push(
        `package.json:${override.path}: xmldom override ${override.specification} is affected`,
      );
    }
  }

  const epubSpecification = manifest.dependencies?.epubjs;
  const lockedEpub = lockfile.packages?.['node_modules/epubjs'];
  const xmldomSpecification = lockedEpub?.dependencies?.['@xmldom/xmldom'];
  if (
    typeof epubSpecification === 'string' &&
    typeof xmldomSpecification === 'string'
  ) {
    if (!dependencyRangeExcludesAffectedVersions(xmldomSpecification)) {
      const overridePath = `epubjs@${epubSpecification}`;
      const override = manifest.overrides?.[overridePath]?.['@xmldom/xmldom'];
      if (typeof override !== 'string' || !EXACT_RELEASE.test(override)) {
        issues.push(
          `package.json:overrides.${overridePath}.@xmldom/xmldom must pin an exact patched release because epubjs declares ${xmldomSpecification}`,
        );
      } else if (!productionVersions.includes(override)) {
        issues.push(
          `package-lock.json: xmldom production graph does not contain the epubjs override ${override}`,
        );
      }
    }
    if (productionVersions.length === 0) {
      issues.push(
        'package-lock.json: epubjs production graph is missing @xmldom/xmldom',
      );
    }
  }

  return [...new Set(issues)];
}

export function dependencyVersionsByEnvironment(lockfile, dependencyName) {
  const production = new Set();
  const development = new Set();
  for (const [packagePath, metadata] of Object.entries(
    lockfile.packages ?? {},
  )) {
    if (
      packagePath === '' ||
      packageNameFromLockPath(packagePath) !== dependencyName ||
      typeof metadata?.version !== 'string'
    ) {
      continue;
    }
    const versions = metadata.dev === true ? development : production;
    versions.add(metadata.version);
  }
  return {
    production: [...production].sort(compareVersionText),
    development: [...development].sort(compareVersionText),
  };
}

export function productionXmldomVersions(lockfile) {
  const versions = new Set();
  for (const [packagePath, metadata] of Object.entries(
    lockfile.packages ?? {},
  )) {
    if (
      metadata?.dev !== true &&
      packageNameFromLockPath(packagePath) === '@xmldom/xmldom' &&
      typeof metadata.version === 'string'
    ) {
      versions.add(metadata.version);
    }
  }
  return [...versions].sort((left, right) => left.localeCompare(right));
}

export function isAffectedXmldomVersion(version) {
  const match = EXACT_RELEASE.exec(version);
  if (!match) return true;
  const major = Number(match[1]);
  const minor = Number(match[2]);
  const patch = Number(match[3]);
  if (major > 0) return false;
  if (minor < 8) return true;
  if (minor === 8) return patch < 15;
  if (minor === 9) return patch < 12;
  return false;
}

export function isAffectedDompurifyVersion(version) {
  const parsed = parseRelease(version);
  return parsed === null || compareRelease(parsed, [3, 4, 12]) <= 0;
}

export function isAffectedNanoidVersion(version) {
  const parsed = parseRelease(version);
  if (parsed === null) return true;
  return (
    compareRelease(parsed, [3, 3, 17]) < 0 ||
    (compareRelease(parsed, [4, 0, 0]) >= 0 &&
      compareRelease(parsed, [5, 1, 6]) < 0)
  );
}

function dependencyRangeExcludesAffectedVersions(specification) {
  const match = SIMPLE_SAFE_RANGE.exec(specification);
  if (!match) return false;
  const version = `${match[2]}.${match[3]}.${match[4]}`;
  return !isAffectedXmldomVersion(version);
}

function collectXmldomOverrides(overrides, prefix = 'overrides') {
  if (!overrides || typeof overrides !== 'object') return [];
  const found = [];
  for (const [name, value] of Object.entries(overrides)) {
    const currentPath = `${prefix}.${name}`;
    if (name === '@xmldom/xmldom') {
      found.push({ path: currentPath, specification: value });
    }
    if (value && typeof value === 'object') {
      found.push(...collectXmldomOverrides(value, currentPath));
    }
  }
  return found;
}

function parseRelease(version) {
  const match = EXACT_RELEASE.exec(version);
  if (!match) return null;
  return [Number(match[1]), Number(match[2]), Number(match[3])];
}

function compareRelease(left, right) {
  for (let index = 0; index < 3; index += 1) {
    const difference = left[index] - right[index];
    if (difference !== 0) return difference;
  }
  return 0;
}

function compareVersionText(left, right) {
  return left.localeCompare(right, 'en', { numeric: true });
}

function formatVersions(versions) {
  return versions.length > 0 ? versions.join(', ') : 'absent';
}

function packageNameFromLockPath(packagePath) {
  const marker = 'node_modules/';
  const markerIndex = packagePath.lastIndexOf(marker);
  if (markerIndex < 0) return null;
  const segments = packagePath.slice(markerIndex + marker.length).split('/');
  if (segments[0]?.startsWith('@')) return segments.slice(0, 2).join('/');
  return segments[0] ?? null;
}
