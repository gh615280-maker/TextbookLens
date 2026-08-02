import { execFileSync } from 'node:child_process';
import {
  mkdtemp,
  readFile,
  rm,
  stat,
  writeFile,
  mkdir,
} from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const projectRoot = fileURLToPath(new URL('..', import.meta.url));
const scriptPath = fileURLToPath(import.meta.url);

const PROTECTED_PATHS = [
  'EWA_Template_Chinese_Translation.docx',
  'ICFTBA2026_OFRW_Submission_With_Author.docx',
  'add_author_block.py',
  'docx_format_audit',
  'wechat-login-qr-new.png',
  'wechat-login-qr.png',
];

// These values exist only to prove the scanner rejects documented fake samples.
const SELF_TEST_SECRET_SAMPLES = [
  ['sk', 'test-secret'].join('-'),
  ['AIza', 'TestOnlyValue123'].join(''),
  ['ANTHROPIC_API_KEY', 'test-only-value'].join('='),
  ['Authorization:', 'Bearer', 'test-only-value'].join(' '),
];

const CONTENT_RULES = [
  { name: 'openai-key-shape', pattern: /\bsk-[A-Za-z0-9_-]{8,}\b/u },
  { name: 'google-key-shape', pattern: /\bAIza[A-Za-z0-9_-]{8,}\b/u },
  {
    name: 'credential-assignment',
    pattern:
      /\b(?:ANTHROPIC|OPENAI|GOOGLE|GEMINI|DEEPSEEK|KIMI)?_?API_KEY\s*=\s*[^\s#'"`]+/iu,
  },
  {
    name: 'authorization-header',
    pattern: /\bAuthorization\s*:\s*(?:Bearer|Basic)\s+[^\s'"`]+/iu,
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

async function scanFiles(root, relativePaths) {
  const resolvedRoot = path.resolve(root);
  const issues = [];

  for (const relativePath of [...new Set(relativePaths)].sort()) {
    const normalized = normalizeRelativePath(relativePath);
    const protectedMatch = protectedRule(normalized);
    if (protectedMatch) {
      issues.push({ path: normalized, rule: protectedMatch });
      continue;
    }

    const absolutePath = path.resolve(resolvedRoot, normalized);
    const relativeToRoot = path.relative(resolvedRoot, absolutePath);
    if (relativeToRoot.startsWith('..') || path.isAbsolute(relativeToRoot)) {
      issues.push({ path: normalized, rule: 'outside-scan-root' });
      continue;
    }

    let metadata;
    try {
      metadata = await stat(absolutePath);
    } catch {
      continue;
    }
    if (!metadata.isFile()) {
      continue;
    }

    const buffer = await readFile(absolutePath);
    if (buffer.includes(0)) {
      continue;
    }
    let content = buffer.toString('utf8');
    if (path.resolve(absolutePath) === path.resolve(scriptPath)) {
      for (const sample of SELF_TEST_SECRET_SAMPLES) {
        content = content.replaceAll(sample, '[SELF_TEST_SAMPLE]');
      }
    }
    for (const rule of CONTENT_RULES) {
      if (rule.pattern.test(content)) {
        issues.push({ path: normalized, rule: rule.name });
      }
    }
  }

  return issues;
}

function gitPaths() {
  const commands = [
    ['ls-files', '--cached', '-z'],
    ['diff', '--cached', '--name-only', '--diff-filter=ACMR', '-z'],
  ];
  const paths = [];
  for (const args of commands) {
    const output = execFileSync('git', args, {
      cwd: projectRoot,
      encoding: 'utf8',
    });
    paths.push(...output.split('\0').filter(Boolean));
  }
  return paths;
}

async function runSelfTest() {
  const root = await mkdtemp(path.join(tmpdir(), 'textbooklens-sensitive-'));
  try {
    const accepted = ['ordinary-fixture.txt', 'profile.txt'];
    await writeFile(path.join(root, accepted[0]), 'ordinary fixture text');
    await writeFile(
      path.join(root, accepted[1]),
      'provider_profile_id=local-id',
    );
    if ((await scanFiles(root, accepted)).length !== 0) {
      throw new Error('safe fixture self-test failed');
    }

    for (const [index, sample] of SELF_TEST_SECRET_SAMPLES.entries()) {
      const relativePath = `secret-${index}.txt`;
      await writeFile(path.join(root, relativePath), sample);
      if ((await scanFiles(root, [relativePath])).length === 0) {
        throw new Error(`secret rule self-test ${index} failed`);
      }
    }

    for (const [index, protectedPath] of PROTECTED_PATHS.entries()) {
      const relativePath = protectedPath.includes('.')
        ? protectedPath
        : `${protectedPath}/probe.txt`;
      const absolutePath = path.join(root, relativePath);
      await mkdir(path.dirname(absolutePath), { recursive: true });
      await writeFile(absolutePath, `protected probe ${index}`);
      if ((await scanFiles(root, [relativePath])).length === 0) {
        throw new Error(`protected path self-test ${index} failed`);
      }
    }
  } finally {
    await rm(root, { recursive: true, force: true });
  }
  console.log('Sensitive-file guard self-test passed.');
}

if (process.argv.includes('--self-test')) {
  await runSelfTest();
} else {
  const issues = await scanFiles(projectRoot, gitPaths());
  for (const issue of issues) {
    console.error(`${issue.path}: ${issue.rule}`);
  }
  if (issues.length > 0) {
    process.exitCode = 1;
  } else {
    console.log('Sensitive-file guard passed.');
  }
}
