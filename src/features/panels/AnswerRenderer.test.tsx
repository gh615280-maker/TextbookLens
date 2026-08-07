import { render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import {
  AnswerRenderer,
  normalizeProviderMathDelimiters,
  safeMarkdownUrl,
  stripHiddenReasoning,
} from './AnswerRenderer';

describe('AnswerRenderer', () => {
  it('keeps raw HTML and unsafe links inert while retaining safe HTTPS and mailto links', () => {
    const { container } = render(
      <AnswerRenderer
        answer={
          '<img src=x onerror=alert(1)> [bad](javascript:alert(1)) [web](https://example.test/path) [mail](mailto:reader@example.test)'
        }
      />,
    );
    expect(container.querySelector('img')).toBeNull();
    expect(container.querySelector('[onerror]')).toBeNull();
    expect(screen.getByText(/<img src=x/)).toBeInTheDocument();
    expect(screen.getByText(/link blocked/)).toBeInTheDocument();
    expect(screen.getByRole('link', { name: 'web' })).toHaveAttribute(
      'href',
      'https://example.test/path',
    );
    expect(screen.getByRole('link', { name: 'web' })).toHaveAttribute(
      'rel',
      expect.stringContaining('noopener'),
    );
    expect(screen.getByRole('link', { name: 'mail' })).toHaveAttribute(
      'href',
      'mailto:reader@example.test',
    );
    expect(safeMarkdownUrl('https://example.test')).toBe(
      'https://example.test/',
    );
    for (const unsafe of [
      'javascript:alert(1)',
      'data:text/html,nope',
      'file:///secret',
      'blob:https://example.test/id',
      'tauri://open',
    ]) {
      expect(safeMarkdownUrl(unsafe)).toBeNull();
    }
  });

  it('renders Markdown and closed math but leaves malformed streaming math readable', () => {
    const { container } = render(
      <AnswerRenderer
        answer={
          '# Heading\n\nA *word* and **strong**.\n\n- one\n- two\n\n`inline`\n\n$x^2$\n\n$$\\frac{1}{2}$$\n\n$\\notARealCommand$\n\n$\\frac{'
        }
      />,
    );
    expect(screen.getByRole('heading', { name: 'Heading' })).toBeVisible();
    expect(screen.getByText('word').tagName).toBe('EM');
    expect(screen.getByText('strong').tagName).toBe('STRONG');
    expect(screen.getAllByRole('listitem')).toHaveLength(2);
    expect(screen.getByText('inline').tagName).toBe('CODE');
    expect(container.querySelectorAll('.katex').length).toBeGreaterThanOrEqual(
      2,
    );
    expect(screen.getAllByText(/notARealCommand/).length).toBeGreaterThan(0);
    expect(screen.getByText(/\$\\frac\{/)).toBeInTheDocument();
  });

  it('renders provider parenthesis and bracket math delimiters without rewriting code', () => {
    const answer =
      '行内公式 \\(x^2 + 1\\)，展示公式：\\[\\frac{1}{2}\\]\n\n`\\(inline code\\)`\n\n```tex\n\\[fenced code\\]\n```';
    const { container } = render(<AnswerRenderer answer={answer} />);
    expect(container.querySelectorAll('.katex')).toHaveLength(2);
    expect(screen.getByText('\\(inline code\\)')).toBeInTheDocument();
    expect(screen.getByText(/\\\[fenced code\\\]/)).toBeInTheDocument();
    expect(normalizeProviderMathDelimiters('streaming \\(x + 1')).toBe(
      'streaming \\(x + 1',
    );
  });

  it('omits Markdown images without a request and bounds oversized fenced code', () => {
    const fetchSpy = vi.fn();
    vi.stubGlobal('fetch', fetchSpy);
    const answer = `![text](https://example.test/image.png)\n\`\`\`\n${'a'.repeat(9_000)}\n\`\`\``;
    const { container } = render(<AnswerRenderer answer={answer} />);
    expect(container.querySelector('img')).toBeNull();
    expect(fetchSpy).not.toHaveBeenCalled();
    expect(container.querySelector('code')?.textContent).toContain(
      '[code block truncated]',
    );
    expect(container.querySelector('code')?.textContent?.length).toBeLessThan(
      8_300,
    );
    vi.unstubAllGlobals();
  });

  it('removes complete and partial provider reasoning blocks before Markdown parsing', () => {
    const { rerender } = render(
      <AnswerRenderer
        answer={
          'Visible before. <think>private chain</think> Visible after. <analysis>nested <reasoning>private</reasoning></analysis>'
        }
      />,
    );
    expect(screen.getByText(/Visible before/)).toHaveTextContent(
      'Visible before. Visible after.',
    );
    expect(screen.queryByText(/private/)).not.toBeInTheDocument();
    rerender(
      <AnswerRenderer answer="Durable prefix. <think>streaming private" />,
    );
    expect(screen.getByText('Durable prefix.')).toBeVisible();
    expect(screen.queryByText(/streaming private/)).not.toBeInTheDocument();
    expect(stripHiddenReasoning('safe </think> tail')).toBe('safe  tail');
  });
});
