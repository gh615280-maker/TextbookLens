import { describe, expect, it } from 'vitest';

import {
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
