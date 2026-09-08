import { constants as fsConstants } from 'node:fs';
import { lstat, open, readFile, readdir, realpath } from 'node:fs/promises';
import { homedir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import { DEFAULT_SCAN_LIMITS, scanFile } from './check-sensitive-files.mjs';

const scriptPath = import.meta.url.startsWith('file:')
  ? fileURLToPath(import.meta.url)
  : path.join(process.cwd(), 'scripts', 'audit-release-artifacts.mjs');
const projectRoot = path.resolve(path.dirname(scriptPath), '..');

export const DEFAULT_AUDIT_LIMITS = Object.freeze({
  ...DEFAULT_SCAN_LIMITS,
  // Installers may contain the official offline WebView2 runtime.
  maxFileBytes: 512 * 1024 * 1024,
  maxFiles: 12_000,
  maxDepth: 24,
  maxTotalBytes: 2 * 1024 * 1024 * 1024,
});

const OFFICIAL_PROVIDER_ORIGINS = Object.freeze([
  'https://api.openai.com/',
  'https://generativelanguage.googleapis.com/',
  'https://api.anthropic.com/',
  'https://api.deepseek.com/',
  'https://api.moonshot.cn/v1/',
  'https://api.moonshot.ai/v1/',
]);

const DEFAULT_TARGETS = Object.freeze([
  {
    label: 'frontend-dist',
    kind: 'root',
    path: path.join(projectRoot, 'dist'),
    optional: true,
  },
  {
    label: 'fixture-corpus',
    kind: 'root',
    path: path.join(projectRoot, 'fixtures'),
  },
  {
    label: 'tauri-resources',
    kind: 'root',
    path: path.join(projectRoot, 'src-tauri', 'resources'),
  },
  {
    label: 'migrations',
    kind: 'root',
    path: path.join(projectRoot, 'src-tauri', 'migrations'),
  },
  {
    label: 'npm-manifest',
    kind: 'file',
    path: path.join(projectRoot, 'package.json'),
  },
  {
    label: 'npm-lock',
    kind: 'file',
    path: path.join(projectRoot, 'package-lock.json'),
  },
  {
    label: 'cargo-manifest',
    kind: 'file',
    path: path.join(projectRoot, 'src-tauri', 'Cargo.toml'),
  },
  {
    label: 'cargo-lock',
    kind: 'file',
    path: path.join(projectRoot, 'src-tauri', 'Cargo.lock'),
  },
  {
    label: 'cargo-policy',
    kind: 'file',
    path: path.join(projectRoot, 'deny.toml'),
  },
  {
    label: 'installer-bundle',
    kind: 'root',
    path: path.join(projectRoot, 'src-tauri', 'target', 'release', 'bundle'),
    optional: true,
    task7Gate: true,
  },
]);

export class ArtifactAuditError extends Error {
  constructor(code, message) {
    super(message);
    this.name = 'ArtifactAuditError';
    this.code = code;
  }
}

function fail(code, message) {
  throw new ArtifactAuditError(code, message);
}

function normalizedCase(value) {
  const normalized = path.normalize(value);
  return process.platform === 'win32'
    ? normalized.toLocaleLowerCase('en-US')
    : normalized;
}

function samePath(left, right) {
  return normalizedCase(left) === normalizedCase(right);
}

function isWithin(parent, child) {
  const relative = path.relative(parent, child);
  return (
    relative !== '' && !relative.startsWith('..') && !path.isAbsolute(relative)
  );
}

function hasControlCharacters(value) {
  return [...value].some((character) => {
    const codePoint = character.codePointAt(0) ?? 0;
    return codePoint <= 0x1f || codePoint === 0x7f;
  });
}

function validateInputSyntax(rawPath, kind) {
  if (
    typeof rawPath !== 'string' ||
    rawPath.trim() !== rawPath ||
    rawPath === '' ||
    hasControlCharacters(rawPath)
  ) {
    fail('INVALID_PATH', 'artifact path must be a nonempty unpadded string');
  }
  if (/^(?:\\\\|\/\/|\\\\[?.]\\)/u.test(rawPath)) {
    fail('UNSAFE_PATH_KIND', 'UNC and device paths are not accepted');
  }
  if (/[*?[\]{}]/u.test(rawPath) || /%[^%]+%|\$\{|\$[A-Za-z_]/u.test(rawPath)) {
    fail(
      'UNEXPANDED_PATH',
      'artifact path must not contain glob or environment syntax',
    );
  }
  const segments = rawPath.split(/[\\/]+/u);
  if (segments.includes('..') || segments.includes('.')) {
    fail('PATH_TRAVERSAL', 'artifact path must not contain dot segments');
  }
  if (!path.isAbsolute(rawPath)) {
    fail('RELATIVE_PATH', 'artifact path must be absolute');
  }
  const resolved = path.resolve(rawPath);
  const parsed = path.parse(resolved);
  if (samePath(resolved, parsed.root)) {
    fail('BROAD_ROOT', 'filesystem roots are not audit targets');
  }
  if (samePath(resolved, projectRoot) || samePath(resolved, homedir())) {
    fail('BROAD_ROOT', 'workspace and home roots are not audit targets');
  }
  if (kind === 'root') {
    const relativeFromRoot = path.relative(parsed.root, resolved);
    if (relativeFromRoot.split(path.sep).filter(Boolean).length < 2) {
      fail('BROAD_ROOT', 'artifact roots must be narrowly scoped');
    }
  }
  for (const segment of segments) {
    if (/^(?:con|prn|aux|nul|com[1-9]|lpt[1-9])(?:\..*)?$/iu.test(segment)) {
      fail('DEVICE_NAME', 'Windows device-name paths are not accepted');
    }
  }
  return resolved;
}

async function inspectExistingComponents(absolutePath) {
  const parsed = path.parse(absolutePath);
  const relative = path.relative(parsed.root, absolutePath);
  let current = parsed.root;
  for (const segment of relative.split(path.sep).filter(Boolean)) {
    current = path.join(current, segment);
    let metadata;
    try {
      metadata = await lstat(current, { bigint: true });
    } catch (error) {
      if (error?.code === 'ENOENT') return;
      throw error;
    }
    if (metadata.isSymbolicLink()) {
      fail(
        'REPARSE_POINT',
        'artifact paths must not traverse links or reparse points',
      );
    }
  }
}

function identity(metadata) {
  return `${metadata.dev}:${metadata.ino}`;
}

function unchanged(left, right) {
  return (
    left.dev === right.dev &&
    left.ino === right.ino &&
    left.size === right.size &&
    left.mtimeNs === right.mtimeNs &&
    left.ctimeNs === right.ctimeNs &&
    left.nlink === right.nlink
  );
}

async function validateExistingTarget(target) {
  await inspectExistingComponents(target.path);
  const canonical = await realpath(target.path);
  if (!samePath(canonical, target.path)) {
    fail(
      'REPARSE_POINT',
      'artifact target canonicalizes through a link or alias',
    );
  }
  const metadata = await lstat(target.path, { bigint: true });
  if (metadata.isSymbolicLink()) {
    fail(
      'REPARSE_POINT',
      'artifact targets must not be links or reparse points',
    );
  }
  if (target.kind === 'file' && !metadata.isFile()) {
    fail('WRONG_TARGET_KIND', 'expected artifact file');
  }
  if (target.kind === 'root' && !metadata.isDirectory()) {
    fail('WRONG_TARGET_KIND', 'expected artifact directory');
  }
  return { canonical, metadata };
}

async function pathExists(absolutePath) {
  try {
    await lstat(absolutePath);
    return true;
  } catch (error) {
    if (error?.code === 'ENOENT') return false;
    throw error;
  }
}

async function enumerateRoot(root, limits) {
  const files = [];
  const queue = [{ directory: root, depth: 0 }];
  while (queue.length > 0) {
    const { directory, depth } = queue.shift();
    if (depth > limits.maxDepth) {
      fail(
        'DEPTH_LIMIT',
        'artifact directory depth exceeds the configured limit',
      );
    }
    const before = await lstat(directory, { bigint: true });
    if (before.isSymbolicLink() || !before.isDirectory()) {
      fail('REPARSE_POINT', 'artifact directory changed into an unsafe entry');
    }
    const entries = await readdir(directory, { withFileTypes: true });
    entries.sort((left, right) => left.name.localeCompare(right.name));
    for (const entry of entries) {
      const entryPath = path.join(directory, entry.name);
      const metadata = await lstat(entryPath, { bigint: true });
      if (metadata.isSymbolicLink()) {
        fail('REPARSE_POINT', 'artifact tree contains a link or reparse point');
      }
      if (metadata.isDirectory()) {
        queue.push({ directory: entryPath, depth: depth + 1 });
      } else if (metadata.isFile()) {
        files.push(entryPath);
        if (files.length > limits.maxFiles) {
          fail(
            'FILE_COUNT_LIMIT',
            'artifact file count exceeds the configured limit',
          );
        }
      } else {
        fail('UNSUPPORTED_ENTRY', 'artifact tree contains a non-file entry');
      }
    }
    const after = await lstat(directory, { bigint: true });
    if (!unchanged(before, after)) {
      fail('RACE_DETECTED', 'artifact directory changed during enumeration');
    }
  }
  return files;
}

async function verifyReadableFile(filePath, seenIdentities, limits) {
  const metadata = await lstat(filePath, { bigint: true });
  if (!metadata.isFile() || metadata.isSymbolicLink()) {
    fail('UNSAFE_FILE', 'artifact file is not a stable regular file');
  }
  if (metadata.nlink > 1n) {
    fail('HARDLINK', 'hard-linked artifact files are rejected');
  }
  if (metadata.size > BigInt(limits.maxFileBytes)) {
    fail('FILE_SIZE_LIMIT', 'artifact file exceeds the configured limit');
  }
  const canonical = await realpath(filePath);
  if (!samePath(canonical, filePath)) {
    fail(
      'REPARSE_POINT',
      'artifact file canonicalizes through a link or alias',
    );
  }
  const fileIdentity = identity(metadata);
  if (seenIdentities.has(fileIdentity)) {
    fail(
      'DUPLICATE_FILE',
      'duplicate artifact identity was supplied or enumerated',
    );
  }
  seenIdentities.add(fileIdentity);

  const handle = await open(
    filePath,
    fsConstants.O_RDONLY | (fsConstants.O_NOFOLLOW ?? 0),
  );
  try {
    const opened = await handle.stat({ bigint: true });
    if (!unchanged(metadata, opened)) {
      fail('RACE_DETECTED', 'artifact identity changed before scanning');
    }
  } finally {
    await handle.close();
  }
  return metadata;
}

const EXACT_CARGO_VERSION =
  /^=(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$/u;

export function auditCargoManifestPins(source) {
  const issues = [];
  let dependencySection = false;
  for (const rawLine of source.split(/\r?\n/u)) {
    const line = rawLine.replace(/\s+#.*$/u, '').trim();
    if (line.startsWith('[')) {
      if (
        /^\[(?:(?:build-|dev-)?dependencies|target\..+\.(?:build-|dev-)?dependencies)\./u.test(
          line,
        )
      ) {
        issues.push(
          'per-dependency Cargo tables are not permitted; use one-line exact pins',
        );
        dependencySection = false;
        continue;
      }
      dependencySection =
        /^\[(?:(?:build-|dev-)?dependencies|target\..+\.(?:build-|dev-)?dependencies)\]$/u.test(
          line,
        );
      continue;
    }
    if (!dependencySection || line === '' || line.startsWith('#')) continue;
    const separator = line.indexOf('=');
    if (separator <= 0) {
      issues.push('dependency declaration is not a key/value pair');
      continue;
    }
    const dependency = line.slice(0, separator).trim();
    const declaration = line.slice(separator + 1).trim();
    const simple = declaration.match(/^"([^"]+)"$/u)?.[1];
    const table = declaration.match(/\bversion\s*=\s*"([^"]+)"/u)?.[1];
    const version = simple ?? table;
    if (!version || !EXACT_CARGO_VERSION.test(version)) {
      issues.push(
        `${dependency}: direct Cargo dependency must use an exact =version pin`,
      );
    }
    if (/\b(?:git|path|registry|workspace)\s*=/u.test(declaration)) {
      issues.push(
        `${dependency}: direct Cargo dependency must come from the locked registry`,
      );
    }
  }
  return issues;
}

async function auditNetworkBoundary() {
  const transportPath = path.join(
    projectRoot,
    'src-tauri',
    'src',
    'ai',
    'transport.rs',
  );
  const regionPath = path.join(
    projectRoot,
    'src-tauri',
    'src',
    'ai',
    'kimi_region.rs',
  );
  const extractionPath = path.join(
    projectRoot,
    'src-tauri',
    'src',
    'extraction',
    'kimi_files.rs',
  );
  const cargoManifestPath = path.join(projectRoot, 'src-tauri', 'Cargo.toml');
  const [transport, region, extraction, cargoManifest] = await Promise.all([
    readFile(transportPath, 'utf8'),
    readFile(regionPath, 'utf8'),
    readFile(extractionPath, 'utf8'),
    readFile(cargoManifestPath, 'utf8'),
  ]);
  const cargoPinIssues = auditCargoManifestPins(cargoManifest);
  if (cargoPinIssues.length > 0) {
    fail('CARGO_DIRECT_PIN_DRIFT', cargoPinIssues.join('; '));
  }
  const actualOrigins = new Set(
    `${transport}\n${region}`.match(/https:\/\/[^"'\s]+\//gu) ?? [],
  );
  const expectedOrigins = new Set(OFFICIAL_PROVIDER_ORIGINS);
  if (
    actualOrigins.size !== expectedOrigins.size ||
    [...actualOrigins].some((origin) => !expectedOrigins.has(origin))
  ) {
    fail(
      'PRODUCTION_ORIGIN_DRIFT',
      'production provider origin allowlist drifted',
    );
  }
  for (const [label, source] of [
    ['provider transport', transport],
    ['Kimi Files transport', extraction],
  ]) {
    for (const required of [
      '.no_proxy()',
      '.tls_backend_rustls()',
      'Policy::none()',
      '.referer(false)',
      'retry::never()',
    ]) {
      if (!source.includes(required)) {
        fail('TRANSPORT_POLICY_DRIFT', `${label} is missing ${required}`);
      }
    }
  }

  const frontendFiles = await enumerateRoot(path.join(projectRoot, 'src'), {
    ...DEFAULT_AUDIT_LIMITS,
    maxFiles: 4_000,
  });
  for (const file of frontendFiles) {
    if (/\.(?:test|spec)\.[cm]?[jt]sx?$/u.test(file)) continue;
    const source = await readFile(file, 'utf8');
    if (/\b(?:fetch|XMLHttpRequest|WebSocket|EventSource)\s*\(/u.test(source)) {
      fail(
        'BROWSER_NETWORK_PATH',
        'frontend production source opens a network path',
      );
    }
    if (
      /sendBeacon|telemetry|analytics|sentry|update[_ -]?channel/iu.test(source)
    ) {
      fail(
        'UNSPECIFIED_REMOTE_FEATURE',
        'frontend contains an unspecified remote feature',
      );
    }
  }
  return { origins: [...actualOrigins].sort() };
}

export async function auditArtifacts(targets, options = {}) {
  const limits = { ...DEFAULT_AUDIT_LIMITS, ...options.limits };
  const normalizedTargets = targets.map((target) => ({
    ...target,
    path: validateInputSyntax(target.path, target.kind),
  }));
  const targetPaths = new Set();
  for (const target of normalizedTargets) {
    const key = normalizedCase(target.path);
    if (targetPaths.has(key))
      fail('DUPLICATE_TARGET', 'duplicate artifact target');
    targetPaths.add(key);
  }

  const reports = [];
  const findings = [];
  const seenIdentities = new Set();
  let fileCount = 0;
  let totalBytes = 0;

  for (const target of normalizedTargets) {
    if (!(await pathExists(target.path))) {
      if (!target.optional)
        fail('NOT_FOUND', `required target is not present: ${target.label}`);
      reports.push({
        label: target.label,
        status: 'NOT PRESENT',
        task7Gate: target.task7Gate === true,
      });
      continue;
    }
    const validatedTarget = await validateExistingTarget(target);
    const files =
      target.kind === 'file'
        ? [target.path]
        : await enumerateRoot(target.path, limits);
    let targetBytes = 0;
    for (const file of files) {
      const metadata = await verifyReadableFile(file, seenIdentities, limits);
      fileCount += 1;
      totalBytes += Number(metadata.size);
      targetBytes += Number(metadata.size);
      if (fileCount > limits.maxFiles)
        fail('FILE_COUNT_LIMIT', 'total file count limit exceeded');
      if (totalBytes > limits.maxTotalBytes)
        fail('TOTAL_SIZE_LIMIT', 'total byte limit exceeded');
      const displayPath = isWithin(projectRoot, file)
        ? path.relative(projectRoot, file).replaceAll('\\', '/')
        : `${target.label}/${path.basename(file)}`;
      const fileFindings = await scanFile(file, displayPath, {
        ...limits,
        afterOpen: options.afterOpen,
      });
      if (
        fileFindings.some(
          (finding) => finding.rule === 'file-changed-during-scan',
        )
      ) {
        fail('RACE_DETECTED', 'artifact changed while it was being scanned');
      }
      const afterScan = await lstat(file, { bigint: true });
      if (!unchanged(metadata, afterScan)) {
        fail('RACE_DETECTED', 'artifact identity changed across scanning');
      }
      findings.push(...fileFindings);
    }
    const targetAfter = await lstat(target.path, { bigint: true });
    if (!unchanged(validatedTarget.metadata, targetAfter)) {
      fail('RACE_DETECTED', 'artifact target changed across scanning');
    }
    reports.push({
      label: target.label,
      status: 'SCANNED',
      files: files.length,
      bytes: targetBytes,
    });
  }

  if (fileCount === 0) {
    fail('NO_PRESENT_TARGETS', 'no present artifact files were scanned');
  }

  const network = options.skipNetworkAudit
    ? null
    : await auditNetworkBoundary();
  return { reports, findings, fileCount, totalBytes, network };
}

function parseLabelledPath(value, kind, optional = false) {
  const separator = value.indexOf('=');
  if (separator <= 0 || separator === value.length - 1) {
    fail('INVALID_ARGUMENT', 'expected label=absolute-path');
  }
  const label = value.slice(0, separator);
  if (!/^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$/u.test(label)) {
    fail('INVALID_ARGUMENT', 'artifact label must be a short safe identifier');
  }
  return {
    label,
    kind,
    path: value.slice(separator + 1),
    optional,
  };
}

export function parseArguments(argv) {
  const targets = [];
  let useDefaults = true;
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    if (argument === '--no-defaults') {
      useDefaults = false;
      continue;
    }
    const value = argv[index + 1];
    if (!value || value.startsWith('--')) {
      fail('INVALID_ARGUMENT', `${argument} requires a value`);
    }
    if (argument === '--file') {
      targets.push({
        label: `file-${targets.length + 1}`,
        kind: 'file',
        path: value,
      });
    } else if (argument === '--root') {
      targets.push({
        label: `root-${targets.length + 1}`,
        kind: 'root',
        path: value,
      });
    } else if (argument === '--expect-file') {
      targets.push(parseLabelledPath(value, 'file', true));
    } else if (argument === '--expect-root') {
      targets.push(parseLabelledPath(value, 'root', true));
    } else {
      fail('INVALID_ARGUMENT', `unknown argument: ${argument}`);
    }
    index += 1;
  }
  const combined = [...(useDefaults ? DEFAULT_TARGETS : []), ...targets];
  if (combined.length === 0) {
    fail('NO_TARGETS', 'at least one explicit artifact target is required');
  }
  return combined;
}

async function main() {
  const targets = parseArguments(process.argv.slice(2));
  const result = await auditArtifacts(targets);
  for (const report of result.reports) {
    if (report.status === 'NOT PRESENT') {
      const suffix = report.task7Gate
        ? '; Task 7 installer rescan required'
        : '';
      console.log(`${report.label}: NOT PRESENT${suffix}`);
    } else {
      console.log(
        `${report.label}: SCANNED files=${report.files} bytes=${report.bytes}`,
      );
    }
  }
  for (const finding of result.findings) {
    console.error(`${finding.path}: ${finding.rule}`);
  }
  if (result.findings.length > 0) {
    console.error(
      `Release artifact audit found ${result.findings.length} prohibited value(s).`,
    );
    process.exitCode = 1;
    return;
  }
  console.log(
    `Release artifact audit passed for ${result.fileCount} files (${result.totalBytes} bytes); official origins=${result.network.origins.length}.`,
  );
}

if (
  process.argv[1] &&
  path.resolve(process.argv[1]) === path.resolve(scriptPath)
) {
  main().catch((error) => {
    const code =
      error instanceof ArtifactAuditError ? error.code : 'UNEXPECTED_ERROR';
    console.error(
      `Release artifact audit failed safely [${code}]: ${error.message}`,
    );
    process.exitCode = 2;
  });
}
