import { spawnSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const moduleFile = import.meta.url.startsWith('file:')
  ? fileURLToPath(import.meta.url)
  : path.join(process.cwd(), 'scripts', 'check-npm-licenses.mjs');
const projectRoot = path.resolve(path.dirname(moduleFile), '..');
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
const packageLicenseAliases = new Map([
  [
    'duck@0.1.12',
    new Map([
      // The package metadata says "BSD*"; its bundled LICENSE is the
      // two-clause BSD text. Keep this normalization version-specific.
      ['BSD*', 'BSD-2-Clause'],
    ]),
  ],
]);
const rootManifest = JSON.parse(
  readFileSync(path.join(projectRoot, 'package.json'), 'utf8'),
);
const rootPackageKey = `${rootManifest.name}@${rootManifest.version}`;

if (process.argv[1] && path.resolve(process.argv[1]) === moduleFile) {
  runLicenseCheck();
}

function runLicenseCheck() {
  const result = spawnSync(
    process.execPath,
    [licenseChecker, '--production', '--json', '--start', projectRoot],
    { cwd: projectRoot, encoding: 'utf8' },
  );
  if (result.error) throw result.error;
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
    for (const reportedExpression of expressions) {
      const expression = normalizeLicenseExpression(
        dependency,
        reportedExpression,
      );
      try {
        const packageAllowed = packageExceptions.get(dependency) ?? new Set();
        if (!isLicenseExpressionAllowed(expression, allowed, packageAllowed)) {
          const permitted = new Set([...allowed, ...packageAllowed]);
          const denied = collectLicenseIdentifiers(
            parseSpdxExpression(expression),
          ).filter((identifier) => !permitted.has(identifier));
          rejected.push({
            dependency,
            expression: reportedExpression,
            denied: denied.length > 0 ? denied : ['expression-not-allowed'],
          });
        }
      } catch {
        rejected.push({
          dependency,
          expression: reportedExpression,
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
}

export function normalizeLicenseExpression(dependency, expression) {
  if (typeof expression !== 'string') return expression;
  return packageLicenseAliases.get(dependency)?.get(expression) ?? expression;
}

export function isLicenseExpressionAllowed(
  expression,
  globallyAllowed,
  packageAllowed = new Set(),
) {
  const permitted = new Set([...globallyAllowed, ...packageAllowed]);
  return evaluateLicenseExpression(parseSpdxExpression(expression), permitted);
}

export function parseSpdxExpression(expression) {
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
  const peek = () => tokens[position];
  const take = () => tokens[position++];

  function primary() {
    if (peek() === '(') {
      take();
      const node = expressionNode();
      if (take() !== ')') throw new Error('missing closing parenthesis');
      return node;
    }
    const identifier = take();
    if (!identifier || ['AND', 'OR', 'WITH', ')'].includes(identifier)) {
      throw new Error('expected license identifier');
    }
    let exception;
    if (peek() === 'WITH') {
      take();
      exception = take();
      if (!exception || ['AND', 'OR', 'WITH', '(', ')'].includes(exception)) {
        throw new Error('expected SPDX exception');
      }
    }
    return { type: 'license', identifier, exception };
  }

  function andNode() {
    const nodes = [primary()];
    while (peek() === 'AND') {
      take();
      nodes.push(primary());
    }
    return nodes.length === 1 ? nodes[0] : { type: 'and', nodes };
  }

  function expressionNode() {
    const nodes = [andNode()];
    while (peek() === 'OR') {
      take();
      nodes.push(andNode());
    }
    return nodes.length === 1 ? nodes[0] : { type: 'or', nodes };
  }

  const root = expressionNode();
  if (position !== tokens.length) throw new Error('trailing SPDX tokens');
  return root;
}

function evaluateLicenseExpression(node, permitted) {
  if (node.type === 'license') {
    return (
      permitted.has(node.identifier) &&
      (node.exception === undefined || permitted.has(node.exception))
    );
  }
  if (node.type === 'and') {
    return node.nodes.every((child) =>
      evaluateLicenseExpression(child, permitted),
    );
  }
  return node.nodes.some((child) =>
    evaluateLicenseExpression(child, permitted),
  );
}

function collectLicenseIdentifiers(node) {
  if (node.type === 'license') {
    return node.exception
      ? [node.identifier, node.exception]
      : [node.identifier];
  }
  return node.nodes.flatMap(collectLicenseIdentifiers);
}
