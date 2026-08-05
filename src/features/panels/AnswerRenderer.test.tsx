import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { AnswerRenderer, safeMarkdownUrl } from './AnswerRenderer';

describe('AnswerRenderer', () => {
  it('renders unsafe URLs, raw HTML and incomplete streaming delimiters as inert text', () => {
    const { container } = render(
      <AnswerRenderer
        answer={
          '<img src=x onerror=alert(1)> [bad](javascript:alert(1)) $\\frac{'
        }
      />,
    );
    expect(container.querySelector('img')).toBeNull();
    expect(container.querySelector('a')).toBeNull();
    expect(screen.getByText(/<img src=x/)).toBeInTheDocument();
    expect(screen.getByText(/\$\\frac\{/)).toBeInTheDocument();
    expect(safeMarkdownUrl('https://example.test')).toBeNull();
    expect(safeMarkdownUrl('data:text/html,nope')).toBeNull();
    expect(safeMarkdownUrl('file:///secret')).toBeNull();
  });

  it('omits Markdown images and bounds oversized code blocks', () => {
    const answer = `![text](https://example.test/image.png)\n\`\`\`\n${'a'.repeat(9_000)}\n\`\`\``;
    const { container } = render(<AnswerRenderer answer={answer} />);
    expect(container.querySelector('img')).toBeNull();
    expect(screen.getByText('[image omitted]')).toBeInTheDocument();
    expect(screen.getByText(/code block truncated/)).toBeInTheDocument();
  });
});
