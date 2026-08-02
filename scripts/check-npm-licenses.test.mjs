import assert from 'node:assert/strict';
import test from 'node:test';

import {
  isLicenseExpressionAllowed,
  normalizeLicenseExpression,
  parseSpdxExpression,
} from './check-npm-licenses.mjs';

const allowed = new Set(['Apache-2.0', 'MIT', 'BSD-2-Clause']);

test('SPDX OR succeeds when at least one branch is allowed', () => {
  assert.equal(
    isLicenseExpressionAllowed('(MIT OR GPL-3.0-or-later)', allowed),
    true,
  );
  assert.equal(
    isLicenseExpressionAllowed('(MPL-2.0 OR Apache-2.0)', allowed),
    true,
  );
});

test('SPDX AND requires every branch and preserves precedence', () => {
  assert.equal(
    isLicenseExpressionAllowed('MIT AND GPL-3.0-or-later', allowed),
    false,
  );
  assert.equal(
    isLicenseExpressionAllowed(
      'MIT OR (Apache-2.0 AND GPL-3.0-or-later)',
      allowed,
    ),
    true,
  );
  assert.equal(
    isLicenseExpressionAllowed(
      'GPL-3.0-or-later OR (Apache-2.0 AND MPL-2.0)',
      allowed,
    ),
    false,
  );
});

test('WITH exceptions require an explicit package-level allowance', () => {
  const expression = 'Apache-2.0 WITH LLVM-exception';
  assert.equal(isLicenseExpressionAllowed(expression, allowed), false);
  assert.equal(
    isLicenseExpressionAllowed(
      expression,
      allowed,
      new Set(['LLVM-exception']),
    ),
    true,
  );
});

test('invalid expressions remain rejected', () => {
  assert.throws(() => parseSpdxExpression('BSD*'));
  assert.throws(() => parseSpdxExpression('MIT OR'));
});

test('duck BSD metadata normalization is exact and version-scoped', () => {
  assert.equal(
    normalizeLicenseExpression('duck@0.1.12', 'BSD*'),
    'BSD-2-Clause',
  );
  assert.equal(normalizeLicenseExpression('duck@0.1.13', 'BSD*'), 'BSD*');
  assert.equal(normalizeLicenseExpression('other@1.0.0', 'BSD*'), 'BSD*');
});
