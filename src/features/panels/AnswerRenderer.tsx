/* eslint-disable react-refresh/only-export-components */

import ReactMarkdown from 'react-markdown';
import rehypeKatex from 'rehype-katex';
import remarkGfm from 'remark-gfm';
import remarkMath from 'remark-math';

import 'katex/dist/katex.min.css';

const MAX_CODE_BLOCK_CODE_POINTS = 8_192;
const BLOCKED_LINK_SUFFIX = ' (link blocked)';

/**
 * Renders a Markdown AST only. `rehype-raw` is intentionally absent: raw
 * provider HTML is discarded before React sees it. KaTeX emits AST nodes,
 * never injected strings; malformed math becomes its selectable error text.
 */
export function AnswerRenderer({ answer }: { answer: string }) {
  return (
    <div className="floating-answer" data-testid="safe-answer-renderer">
      <ReactMarkdown
        components={{
          a({ children, href }) {
            const safeHref = href ? safeMarkdownUrl(href) : null;
            return safeHref ? (
              <a href={safeHref} rel="noreferrer noopener" target="_blank">
                {children}
              </a>
            ) : (
              <span>
                {children}
                {BLOCKED_LINK_SUFFIX}
              </span>
            );
          },
          // Images are omitted rather than fetched, including Markdown images.
          img() {
            return null;
          },
        }}
        rehypePlugins={[
          [
            rehypeKatex,
            { strict: 'ignore', throwOnError: false, trust: false },
          ],
        ]}
        remarkPlugins={[remarkGfm, remarkMath]}
      >
        {boundFencedCode(
          normalizeProviderMathDelimiters(stripHiddenReasoning(answer)),
        )}
      </ReactMarkdown>
    </div>
  );
}

/** Normalizes common provider LaTeX delimiters without rewriting code samples. */
export function normalizeProviderMathDelimiters(answer: string): string {
  const lines = answer.match(/[^\n]*\n|[^\n]+$/gu) ?? [];
  const output: string[] = [];
  let prose = '';
  let fence: { character: string; length: number } | null = null;
  for (const line of lines) {
    const marker = line.match(/^ {0,3}(`{3,}|~{3,})/u)?.[1] ?? null;
    if (fence) {
      output.push(line);
      if (
        marker &&
        marker[0] === fence.character &&
        marker.length >= fence.length
      ) {
        fence = null;
      }
      continue;
    }
    if (marker) {
      output.push(normalizeProseMath(prose), line);
      prose = '';
      fence = { character: marker[0], length: marker.length };
      continue;
    }
    prose += line;
  }
  output.push(normalizeProseMath(prose));
  return output.join('');
}

function normalizeProseMath(value: string): string {
  let output = '';
  let index = 0;
  while (index < value.length) {
    if (value[index] === '`') {
      const length = backtickRun(value, index);
      const marker = '`'.repeat(length);
      const end = value.indexOf(marker, index + length);
      if (end >= 0) {
        output += value.slice(index, end + length);
        index = end + length;
        continue;
      }
    }
    const opening = value.slice(index, index + 2);
    const closing =
      opening === '\\(' ? '\\)' : opening === '\\[' ? '\\]' : null;
    if (closing && value[index - 1] !== '\\') {
      const end = value.indexOf(closing, index + 2);
      if (end >= 0) {
        const delimiter = opening === '\\(' ? '$' : '$$';
        output += `${delimiter}${value.slice(index + 2, end)}${delimiter}`;
        index = end + 2;
        continue;
      }
    }
    output += value[index];
    index += 1;
  }
  return output;
}

function backtickRun(value: string, start: number): number {
  let end = start;
  while (value[end] === '`') end += 1;
  return end - start;
}

/** Provider reasoning tags are never user-visible, including an unfinished stream block. */
export function stripHiddenReasoning(answer: string): string {
  let visible = answer;
  const complete = /<(think|analysis|reasoning)\b[^>]*>[\s\S]*?<\/\1\s*>/giu;
  for (;;) {
    const next = visible.replace(complete, '');
    if (next === visible) break;
    visible = next;
  }
  return visible
    .replace(/<(?:think|analysis|reasoning)\b[^>]*>[\s\S]*$/iu, '')
    .replace(/<\/?(?:think|analysis|reasoning)\b[^>]*>/giu, '');
}

/** HTTPS and mailto are the only external protocols accepted by the panel. */
export function safeMarkdownUrl(value: string): string | null {
  try {
    const parsed = new URL(value);
    if (
      (parsed.protocol !== 'https:' && parsed.protocol !== 'mailto:') ||
      parsed.username ||
      parsed.password
    ) {
      return null;
    }
    return parsed.href;
  } catch {
    return null;
  }
}

/** Limits fenced code before parsing, so a provider cannot create a huge code DOM. */
export function boundFencedCode(answer: string): string {
  const lines = answer.replace(/\r\n?/gu, '\n').split('\n');
  let activeFence: string | null = null;
  let count = 0;
  let truncated = false;
  const bounded: string[] = [];
  for (const line of lines) {
    const fence = line.match(/^\s*(`{3,}|~{3,})/u)?.[1] ?? null;
    if (!activeFence && fence) {
      activeFence = fence;
      count = 0;
      truncated = false;
      bounded.push(line);
      continue;
    }
    if (activeFence && fence && fence[0] === activeFence[0]) {
      if (truncated) bounded.push('[code block truncated]');
      bounded.push(line);
      activeFence = null;
      continue;
    }
    if (!activeFence || truncated) {
      bounded.push(line);
      continue;
    }
    const codePoints = [...line];
    const available = MAX_CODE_BLOCK_CODE_POINTS - count;
    if (codePoints.length <= available) {
      bounded.push(line);
      count += codePoints.length;
    } else {
      bounded.push(codePoints.slice(0, Math.max(0, available)).join(''));
      truncated = true;
    }
  }
  if (activeFence && truncated) bounded.push('[code block truncated]');
  return bounded.join('\n');
}
