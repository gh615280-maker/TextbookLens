import { spawnSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const projectRoot = fileURLToPath(new URL('..', import.meta.url));
const licenseChecker = path.join(
  projectRoot,
  'node_modules/license-checker-rseidelsohn/bin/license-checker-rseidelsohn.js',
);
const allowed = new Set([
  'Apache-2.0',
  'MIT',
  'BSD-2-Clause',
  'BSD-3-Clause',
  'ISC',
  'Unicode-3.0',
  'Zlib',
]);
const packageExceptions = new Map([['tslib@2.8.1', new Set(['0BSD'])]]);
const rootManifest = JSON.parse(
  readFileSync(path.join(projectRoot, 'package.json'), 'utf8'),
);
const rootPackageKey = `${rootManifest.name}@${rootManifest.version}`;

const result = spawnSync(
  process.execPath,
  [licenseChecker, '--production', '--json', '--start', projectRoot],
  { cwd: projectRoot, encoding: 'utf8' },
);
if (result.error) {
  throw result.error;
}
if (result.status !== 0) {
  process.stderr.write(result.stderr);
  process.exit(result.status ?? 1);
}

const packages = JSON.parse(result.stdout);
const rejected = [];
for (const [dependency, metadata] of Object.entries(packages)) {
  if (dependency === rootPackageKey) continue;
  const expressions = Array.isArray(metadata.licenses)
    ? metadata.licenses
    : [metadata.licenses];
  for (const expression of expressions) {
    try {
      const identifiers = parseSpdxExpression(expression);
      const exception = packageExceptions.get(dependency) ?? new Set();
      const denied = identifiers.filter(
        (identifier) => !allowed.has(identifier) && !exception.has(identifier),
      );
      if (denied.length > 0) {
        rejected.push({ dependency, expression, denied });
      }
    } catch {
      rejected.push({
        dependency,
        expression,
        denied: ['invalid-or-non-SPDX'],
      });
    }
  }
}

if (rejected.length > 0) {
  for (const item of rejected) {
    console.error(
      `${item.dependency}: ${String(item.expression)} (${item.denied.join(', ')})`,
    );
  }
  process.exitCode = 1;
} else {
  console.log(
    `npm license policy passed for ${Object.keys(packages).length} packages.`,
  );
}

function parseSpdxExpression(expression) {
  if (typeof expression !== 'string' || expression.trim() === '') {
    throw new Error('missing license expression');
  }
  const tokens = expression.match(
    /\(|\)|\bAND\b|\bOR\b|\bWITH\b|[A-Za-z0-9][A-Za-z0-9.+-]*/gu,
  );
  if (
    !tokens ||
    tokens.join('').toLowerCase() !==
      expression.replace(/\s+/gu, '').toLowerCase()
  ) {
    throw new Error('invalid SPDX syntax');
  }

  let position = 0;
  const identifiers = [];
  const peek = () => tokens[position];
  const take = () => tokens[position++];

  function primary() {
    if (peek() === '(') {
      take();
      expressionNode();
      if (take() !== ')') throw new Error('missing closing parenthesis');
      return;
    }
    const identifier = take();
    if (!identifier || ['AND', 'OR', 'WITH', ')'].includes(identifier)) {
      throw new Error('expected license identifier');
    }
    identifiers.push(identifier);
    if (peek() === 'WITH') {
      take();
      const exception = take();
      if (!exception || ['AND', 'OR', 'WITH', '(', ')'].includes(exception)) {
        throw new Error('expected SPDX exception');
      }
      identifiers.push(exception);
    }
  }

  function andNode() {
    primary();
    while (peek() === 'AND') {
      take();
      primary();
    }
  }

  function expressionNode() {
    andNode();
    while (peek() === 'OR') {
      take();
      andNode();
    }
  }

  expressionNode();
  if (position !== tokens.length) throw new Error('trailing SPDX tokens');
  return identifiers;
}
