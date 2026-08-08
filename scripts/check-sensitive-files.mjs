import { Buffer } from 'node:buffer';
import { execFileSync } from 'node:child_process';
import { constants as fsConstants } from 'node:fs';
import {
  lstat,
  mkdir,
  mkdtemp,
  open,
  realpath,
  rm,
  writeFile,
} from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { gunzipSync } from 'node:zlib';

import JSZip from 'jszip';

const scriptPath = import.meta.url.startsWith('file:')
  ? fileURLToPath(import.meta.url)
  : path.join(process.cwd(), 'scripts', 'check-sensitive-files.mjs');
const projectRoot = path.resolve(path.dirname(scriptPath), '..');

const PROTECTED_PATHS = [
  'EWA_Template_Chinese_Translation.docx',
  'ICFTBA2026_OFRW_Submission_With_Author.docx',
  'add_author_block.py',
  'docx_format_audit',
  'wechat-login-qr-new.png',
  'wechat-login-qr.png',
];

export const SENTINEL_CLASSES = Object.freeze([
  'api_credential',
  'credential_identifier',
  'source_absolute_path',
  'textbook_text',
  'page_image_base64',
  'prompt',
  'teaching_instruction',
  'answer',
  'vendor_body',
  'remote_resource_id',
  'user_note',
  'internal_request_id',
  'internal_profile_id',
  'internal_run_id',
  'internal_attempt_id',
]);

export const DEFAULT_SCAN_LIMITS = Object.freeze({
  chunkBytes: 64 * 1024,
  overlapBytes: 64 * 1024,
  maxFileBytes: 256 * 1024 * 1024,
  maxArchiveEntries: 4_096,
  maxArchiveDepth: 3,
  maxArchiveEntryBytes: 64 * 1024 * 1024,
  maxArchiveExpandedBytes: 512 * 1024 * 1024,
  maxDecodedCandidates: 512,
  maxDecodedBytes: 16 * 1024 * 1024,
});

const FORMAT_CHARACTERS =
  // eslint-disable-next-line no-misleading-character-class -- intentional invisible formatting and filler code points
  /[\u00ad\u034f\u061c\u115f\u1160\u17b4\u17b5\u180b-\u180f\u200b-\u200f\u202a-\u202e\u2060-\u206f\ufeff\uffa0]/gu;
const COMPACT_SEPARATORS = /[\s._:/\\-]+/gu;
const BASE64_TOKEN =
  /(?<![A-Za-z0-9+/_-])[A-Za-z0-9+/_-]{16,}={0,2}(?![A-Za-z0-9+/_=-])/gu;
const HEX_TOKEN = /(?<![A-Fa-f0-9])[A-Fa-f0-9]{32,}(?![A-Fa-f0-9])/gu;

function matrixPrefix() {
  return ['tl', 'p15'].join('');
}

export function makeSentinel(sentinelClass, nonce = '7f3a91c2') {
  if (!SENTINEL_CLASSES.includes(sentinelClass)) {
    throw new Error(`unknown sentinel class: ${sentinelClass}`);
  }
  return ['TL', 'P15', sentinelClass, 'ARTIFACT', 'PROBE', nonce].join('_');
}

function compact(value) {
  return value
    .normalize('NFKC')
    .replace(FORMAT_CHARACTERS, '')
    .toLocaleLowerCase('en-US')
    .replace(COMPACT_SEPARATORS, '');
}

function normalize(value) {
  return value
    .normalize('NFKC')
    .replace(FORMAT_CHARACTERS, '')
    .toLocaleLowerCase('en-US');
}

function sentinelRules() {
  return SENTINEL_CLASSES.map((sentinelClass) => ({
    name: `sentinel:${sentinelClass}`,
    compactNeedle: `${matrixPrefix()}${sentinelClass.replaceAll('_', '')}artifactprobe`,
  }));
}

const GENERIC_RULES = [
  { name: 'openai-compatible-key-shape', pattern: /\bsk-[a-z0-9_-]{12,}\b/gu },
  { name: 'google-key-shape', pattern: /\baiza[a-z0-9_-]{12,}\b/gu },
  {
    name: 'credential-assignment',
    pattern:
      /\b(?:anthropic|openai|google|gemini|deepseek|kimi)?_?api_?key\s*[:=]\s*["']?(?!(?:<|%|\$|\{|\[|replace|example|dummy|test|change|set[-_]))(?=[a-z0-9._-]{16,})(?=[a-z0-9._-]*\d)[a-z0-9][a-z0-9._-]{15,}/gu,
  },
  {
    name: 'authorization-header',
    pattern: /\bauthorization\s*:\s*(?:bearer|basic)\s+[^\s'"`]{8,}/gu,
  },
  {
    name: 'credential-identifier',
    pattern:
      /\btextbooklens\/(?:remote-resource\/)?[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}\b/gu,
  },
  {
    name: 'private-absolute-path',
    pattern:
      /(?:\b[a-z]:[\\/](?:users|documents and settings)[\\/][^\r\n"'<>|]+|\/(?:home|users)\/[^\s\r\n"'<>]+\/[^\r\n"'<>]*)/gu,
  },
];

function normalizeRelativePath(value) {
  return value.replaceAll('\\', '/').replace(/^\.\//u, '');
}

function protectedRule(relativePath) {
  const normalized = normalizeRelativePath(relativePath);
  for (const protectedPath of PROTECTED_PATHS) {
    if (
      normalized === protectedPath ||
      normalized.startsWith(`${protectedPath}/`)
    ) {
      return `protected-path:${protectedPath}`;
    }
  }
  return null;
}

function decodeEscapes(value) {
  return value
    .replace(/\\u\{([0-9a-f]{1,6})\}/giu, (_, digits) =>
      String.fromCodePoint(Number.parseInt(digits, 16)),
    )
    .replace(/\\u([0-9a-f]{4})/giu, (_, digits) =>
      String.fromCodePoint(Number.parseInt(digits, 16)),
    )
    .replace(/\\x([0-9a-f]{2})/giu, (_, digits) =>
      String.fromCodePoint(Number.parseInt(digits, 16)),
    );
}

function decodePercent(value) {
  return value.replace(/(?:%[0-9a-f]{2})+/giu, (encoded) => {
    try {
      return decodeURIComponent(encoded);
    } catch {
      return encoded.replace(/%([0-9a-f]{2})/giu, (_, digits) =>
        String.fromCodePoint(Number.parseInt(digits, 16)),
      );
    }
  });
}

function decodeUtf16Be(buffer) {
  const evenLength = buffer.length - (buffer.length % 2);
  const swapped = Buffer.allocUnsafe(evenLength);
  for (let index = 0; index < evenLength; index += 2) {
    swapped[index] = buffer[index + 1];
    swapped[index + 1] = buffer[index];
  }
  return swapped.toString('utf16le');
}

function looksTextual(value) {
  if (value.length === 0) return false;
  let printable = 0;
  for (const character of value.slice(0, 8_192)) {
    const code = character.codePointAt(0) ?? 0;
    if (
      character === '\n' ||
      character === '\r' ||
      character === '\t' ||
      code >= 0x20
    ) {
      printable += 1;
    }
  }
  return printable / Math.min(value.length, 8_192) >= 0.7;
}

function decodeBase64Token(token) {
  const normalizedToken = token.replaceAll('-', '+').replaceAll('_', '/');
  const padding = '='.repeat((4 - (normalizedToken.length % 4)) % 4);
  try {
    const decoded = Buffer.from(`${normalizedToken}${padding}`, 'base64');
    if (decoded.length < 8) return [];
    const variants = [
      decoded.toString('utf8'),
      decoded.toString('latin1'),
      decoded.toString('utf16le'),
      decodeUtf16Be(decoded),
    ];
    return variants.filter(looksTextual);
  } catch {
    return [];
  }
}

function decodedTextVariants(initial, limits) {
  const queue = [{ value: initial, depth: 0 }];
  const seen = new Set();
  const variants = [];
  let candidates = 0;
  let decodedBytes = 0;

  while (queue.length > 0 && candidates < limits.maxDecodedCandidates) {
    const { value, depth } = queue.shift();
    const key =
      value.length > 1_000_000
        ? `${value.length}:${value.slice(0, 512)}`
        : value;
    if (seen.has(key)) continue;
    seen.add(key);
    variants.push(value);
    candidates += 1;
    if (depth >= 2) continue;

    for (const transformed of [decodeEscapes(value), decodePercent(value)]) {
      if (transformed !== value)
        queue.push({ value: transformed, depth: depth + 1 });
    }

    for (const match of value.matchAll(BASE64_TOKEN)) {
      for (const decoded of decodeBase64Token(match[0])) {
        decodedBytes += Buffer.byteLength(decoded);
        if (decodedBytes > limits.maxDecodedBytes) return variants;
        queue.push({ value: decoded, depth: depth + 1 });
      }
    }

    for (const match of value.matchAll(HEX_TOKEN)) {
      try {
        const decoded = Buffer.from(match[0], 'hex').toString('utf8');
        if (looksTextual(decoded)) {
          decodedBytes += Buffer.byteLength(decoded);
          if (decodedBytes > limits.maxDecodedBytes) return variants;
          queue.push({ value: decoded, depth: depth + 1 });
        }
      } catch {
        // Invalid candidate: the raw representation was already scanned.
      }
    }
  }
  return variants;
}

function scanText(text, location, limits) {
  const issues = [];
  for (const variant of decodedTextVariants(text, limits)) {
    const normalized = normalize(variant);
    const compacted = compact(variant);
    for (const rule of sentinelRules()) {
      if (compacted.includes(rule.compactNeedle)) {
        issues.push({ path: location, rule: rule.name });
      }
    }
    for (const rule of GENERIC_RULES) {
      rule.pattern.lastIndex = 0;
      if (rule.pattern.test(normalized)) {
        issues.push({ path: location, rule: rule.name });
      }
    }
  }
  return issues;
}

export function scanBuffer(buffer, location = '<buffer>', options = {}) {
  const limits = { ...DEFAULT_SCAN_LIMITS, ...options };
  const variants = [
    buffer.toString('utf8'),
    buffer.toString('latin1'),
    buffer.toString('utf16le'),
    decodeUtf16Be(buffer),
  ];
  return deduplicateIssues(
    variants.flatMap((variant) => scanText(variant, location, limits)),
  );
}

function archiveEntryPath(name) {
  const normalized = name.replaceAll('\\', '/');
  if (
    normalized.startsWith('/') ||
    /^[a-z]:\//iu.test(normalized) ||
    normalized.split('/').includes('..')
  ) {
    return null;
  }
  return normalized;
}

async function scanZip(buffer, location, limits, archiveState, depth) {
  const issues = [];
  let archive;
  try {
    archive = await JSZip.loadAsync(buffer);
  } catch {
    return [{ path: location, rule: 'archive-unreadable' }];
  }

  const entries = Object.values(archive.files).sort((left, right) =>
    left.name.localeCompare(right.name),
  );
  archiveState.entries += entries.length;
  if (archiveState.entries > limits.maxArchiveEntries) {
    return [{ path: location, rule: 'archive-entry-limit' }];
  }

  for (const entry of entries) {
    const originalName = entry.unsafeOriginalName ?? entry.name;
    const safeName = archiveEntryPath(originalName);
    if (safeName === null) {
      issues.push({
        path: `${location}!<unsafe-entry>`,
        rule: 'archive-entry-path',
      });
      continue;
    }
    const entryLocation = `${location}!${safeName}`;
    issues.push(
      ...scanBuffer(Buffer.from(originalName, 'utf8'), entryLocation, limits),
    );
    if (entry.dir) continue;
    const remainingExpandedBytes =
      limits.maxArchiveExpandedBytes - archiveState.expandedBytes;
    const entryLimit = Math.min(
      limits.maxArchiveEntryBytes,
      remainingExpandedBytes,
    );
    let content;
    try {
      content = await readZipEntryBounded(entry, entryLimit);
    } catch (error) {
      if (error?.code === 'ARCHIVE_OUTPUT_LIMIT') {
        issues.push({
          path: entryLocation,
          rule:
            remainingExpandedBytes <= limits.maxArchiveEntryBytes
              ? 'archive-expanded-size-limit'
              : 'archive-entry-size-limit',
        });
        break;
      }
      issues.push({ path: entryLocation, rule: 'archive-unreadable' });
      continue;
    }
    archiveState.expandedBytes += content.length;
    issues.push(...scanBuffer(content, entryLocation, limits));
    if (isZip(content) && depth < limits.maxArchiveDepth) {
      issues.push(
        ...(await scanZip(
          content,
          entryLocation,
          limits,
          archiveState,
          depth + 1,
        )),
      );
    } else if (isZip(content)) {
      issues.push({ path: entryLocation, rule: 'archive-depth-limit' });
    }
  }
  return issues;
}

function readZipEntryBounded(entry, maxBytes) {
  return new Promise((resolve, reject) => {
    const chunks = [];
    let bytes = 0;
    let settled = false;
    const stream = entry.internalStream('uint8array');
    const rejectOnce = (error) => {
      if (settled) return;
      settled = true;
      stream.pause();
      reject(error);
    };
    stream.on('data', (chunk) => {
      if (settled) return;
      bytes += chunk.length;
      if (bytes > maxBytes) {
        const error = new Error('archive output limit exceeded');
        error.code = 'ARCHIVE_OUTPUT_LIMIT';
        rejectOnce(error);
        return;
      }
      chunks.push(Buffer.from(chunk));
    });
    stream.on('error', rejectOnce);
    stream.on('end', () => {
      if (settled) return;
      settled = true;
      resolve(Buffer.concat(chunks, bytes));
    });
    stream.resume();
  });
}

function isZip(buffer) {
  return (
    buffer.length >= 4 &&
    buffer[0] === 0x50 &&
    buffer[1] === 0x4b &&
    [0x03, 0x05, 0x07].includes(buffer[2])
  );
}

function isGzip(buffer) {
  return buffer.length >= 2 && buffer[0] === 0x1f && buffer[1] === 0x8b;
}

function sameIdentity(left, right) {
  return (
    left.dev === right.dev &&
    left.ino === right.ino &&
    left.size === right.size &&
    left.mtimeNs === right.mtimeNs &&
    left.ctimeNs === right.ctimeNs &&
    left.nlink === right.nlink
  );
}

export async function scanFile(
  absolutePath,
  location = absolutePath,
  options = {},
) {
  const limits = { ...DEFAULT_SCAN_LIMITS, ...options };
  const metadata = await lstat(absolutePath, { bigint: true });
  if (metadata.isSymbolicLink()) {
    return [{ path: location, rule: 'symbolic-link' }];
  }
  if (!metadata.isFile()) return [];
  if (metadata.size > BigInt(limits.maxFileBytes)) {
    return [{ path: location, rule: 'file-size-limit' }];
  }

  const handle = await open(
    absolutePath,
    fsConstants.O_RDONLY | (fsConstants.O_NOFOLLOW ?? 0),
  );
  const chunks = [];
  const issues = [];
  try {
    const before = await handle.stat({ bigint: true });
    if (typeof options.afterOpen === 'function') {
      await options.afterOpen({ absolutePath, handle });
    }
    let carry = Buffer.alloc(0);
    let position = 0;
    while (position < Number(before.size)) {
      const next = Buffer.allocUnsafe(
        Math.min(limits.chunkBytes, Number(before.size) - position),
      );
      const { bytesRead } = await handle.read(next, 0, next.length, position);
      if (bytesRead === 0) break;
      const chunk = next.subarray(0, bytesRead);
      chunks.push(chunk);
      const window = Buffer.concat([carry, chunk]);
      issues.push(...scanBuffer(window, location, limits));
      carry = window.subarray(Math.max(0, window.length - limits.overlapBytes));
      position += bytesRead;
    }
    const after = await handle.stat({ bigint: true });
    const current = await lstat(absolutePath, { bigint: true });
    if (!sameIdentity(before, after) || !sameIdentity(after, current)) {
      issues.push({ path: location, rule: 'file-changed-during-scan' });
    }
  } finally {
    await handle.close();
  }

  const buffer = Buffer.concat(chunks);
  const archiveState = { entries: 0, expandedBytes: 0 };
  if (isZip(buffer)) {
    issues.push(...(await scanZip(buffer, location, limits, archiveState, 1)));
  } else if (isGzip(buffer)) {
    try {
      const expanded = gunzipSync(buffer, {
        maxOutputLength: limits.maxArchiveExpandedBytes,
      });
      issues.push(...scanBuffer(expanded, `${location}!<gzip>`, limits));
      if (isZip(expanded)) {
        issues.push(
          ...(await scanZip(
            expanded,
            `${location}!<gzip>`,
            limits,
            archiveState,
            1,
          )),
        );
      }
    } catch {
      issues.push({ path: location, rule: 'archive-unreadable' });
    }
  }
  return deduplicateIssues(issues);
}

function deduplicateIssues(issues) {
  const seen = new Set();
  return issues.filter((issue) => {
    const key = `${issue.path}\0${issue.rule}`;
    if (seen.has(key)) return false;
    seen.add(key);
    return true;
  });
}

export async function scanFiles(root, relativePaths, options = {}) {
  const resolvedRoot = path.resolve(root);
  const canonicalRoot = await realpath(resolvedRoot);
  const issues = [];
  let scannedFiles = 0;

  for (const relativePath of [...new Set(relativePaths)].sort()) {
    const normalizedPath = normalizeRelativePath(relativePath);
    const protectedMatch = protectedRule(normalizedPath);
    if (protectedMatch) {
      issues.push({ path: normalizedPath, rule: protectedMatch });
      continue;
    }
    if (
      normalizedPath === '' ||
      path.isAbsolute(normalizedPath) ||
      normalizedPath.split('/').includes('..')
    ) {
      issues.push({ path: normalizedPath, rule: 'outside-scan-root' });
      continue;
    }

    const absolutePath = path.resolve(resolvedRoot, normalizedPath);
    const relativeToRoot = path.relative(resolvedRoot, absolutePath);
    if (relativeToRoot.startsWith('..') || path.isAbsolute(relativeToRoot)) {
      issues.push({ path: normalizedPath, rule: 'outside-scan-root' });
      continue;
    }

    let metadata;
    try {
      metadata = await lstat(absolutePath);
    } catch (error) {
      if (error?.code === 'ENOENT') continue;
      throw error;
    }
    if (metadata.isSymbolicLink()) {
      issues.push({ path: normalizedPath, rule: 'symbolic-link' });
      continue;
    }
    if (!metadata.isFile()) continue;
    const canonicalPath = await realpath(absolutePath);
    const canonicalRelative = path.relative(canonicalRoot, canonicalPath);
    if (
      canonicalRelative.startsWith('..') ||
      path.isAbsolute(canonicalRelative)
    ) {
      issues.push({ path: normalizedPath, rule: 'outside-scan-root' });
      continue;
    }
    scannedFiles += 1;
    issues.push(...(await scanFile(absolutePath, normalizedPath, options)));
  }
  return { issues: deduplicateIssues(issues), scannedFiles };
}

function gitPaths() {
  const commands = [
    ['ls-files', '--cached', '-z'],
    ['diff', '--cached', '--name-only', '--diff-filter=ACMR', '-z'],
    ['ls-files', '--others', '--exclude-standard', '-z'],
  ];
  const paths = [];
  for (const args of commands) {
    const output = execFileSync('git', args, {
      cwd: projectRoot,
      encoding: 'utf8',
      maxBuffer: 64 * 1024 * 1024,
    });
    paths.push(...output.split('\0').filter(Boolean));
  }
  return paths;
}

async function runSelfTest() {
  const root = await mkdtemp(path.join(tmpdir(), 'textbooklens-sensitive-'));
  try {
    const safePaths = ['ordinary-fixture.txt', 'profile.txt'];
    await writeFile(
      path.join(root, safePaths[0]),
      'ordinary answer prompt note profile words',
    );
    await writeFile(
      path.join(root, safePaths[1]),
      'provider_profile_id=local-id',
    );
    const safe = await scanFiles(root, safePaths);
    if (safe.issues.length !== 0)
      throw new Error('safe fixture self-test failed');

    for (const [index, sentinelClass] of SENTINEL_CLASSES.entries()) {
      const relativePath = `matrix-${index}.txt`;
      await writeFile(
        path.join(root, relativePath),
        makeSentinel(sentinelClass),
      );
      const result = await scanFiles(root, [relativePath]);
      if (
        !result.issues.some(
          (issue) => issue.rule === `sentinel:${sentinelClass}`,
        )
      ) {
        throw new Error(`sentinel class self-test failed: ${sentinelClass}`);
      }
    }

    const secretSamples = [
      ['sk', 'syntheticcredential1234'].join('-'),
      ['AIza', 'SyntheticCredential1234'].join(''),
      ['OPENAI_API_KEY', 'synthetic-value-1234'].join('='),
      ['Authorization:', 'Bearer', 'synthetic-value-1234'].join(' '),
    ];
    for (const [index, sample] of secretSamples.entries()) {
      const relativePath = `secret-${index}.txt`;
      await writeFile(path.join(root, relativePath), sample);
      if ((await scanFiles(root, [relativePath])).issues.length === 0) {
        throw new Error(`secret rule self-test failed: ${index}`);
      }
    }

    for (const [index, protectedPath] of PROTECTED_PATHS.entries()) {
      const relativePath = protectedPath.includes('.')
        ? protectedPath
        : `${protectedPath}/probe.txt`;
      const absolutePath = path.join(root, relativePath);
      await mkdir(path.dirname(absolutePath), { recursive: true });
      await writeFile(absolutePath, `protected probe ${index}`);
      if ((await scanFiles(root, [relativePath])).issues.length === 0) {
        throw new Error(`protected path self-test failed: ${index}`);
      }
    }
  } finally {
    await rm(root, { recursive: true, force: true });
  }
  console.log('Sensitive-file guard self-test passed.');
}

async function main() {
  if (process.argv.includes('--self-test')) {
    await runSelfTest();
    return;
  }
  const result = await scanFiles(projectRoot, gitPaths());
  for (const issue of result.issues) {
    console.error(`${issue.path}: ${issue.rule}`);
  }
  if (result.issues.length > 0) {
    process.exitCode = 1;
  } else {
    console.log(
      `Sensitive-file guard passed for ${result.scannedFiles} controlled files.`,
    );
  }
}

if (
  process.argv[1] &&
  path.resolve(process.argv[1]) === path.resolve(scriptPath)
) {
  main().catch((error) => {
    console.error(`Sensitive-file guard failed safely: ${error.message}`);
    process.exitCode = 2;
  });
}
