import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { CitationList } from './CitationList';

describe('CitationList', () => {
  it('shows only supplied safe citation DTO fields', () => {
    render(
      <CitationList
        citations={[{ id: 'TL-C1', label: 'Page 2', quoteable: true }]}
        emptyLabel="None"
      />,
    );
    expect(screen.getByLabelText('Citations')).toHaveTextContent('Page 2');
  });
});
