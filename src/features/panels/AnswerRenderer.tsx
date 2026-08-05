/* eslint-disable react-refresh/only-export-components */

import { Fragment } from 'react';

const MAX_CODE_BLOCK_CODE_POINTS = 8_192;

/**
 * Deliberately conservative Markdown presentation.  It creates React text
 * nodes only: raw HTML, image syntax, and links never become executable DOM.
 * Math is retained as text so a malformed streaming delimiter is safe and
 * readable instead of being passed to an HTML-producing KaTeX renderer.
 */
export function AnswerRenderer({ answer }: { answer: string }) {
  return (
    <div className="floating-answer" data-testid="safe-answer-renderer">
      {toBlocks(answer).map((block, index) => (
        <Fragment key={`${block.kind}-${index}`}>{renderBlock(block)}</Fragment>
      ))}
    </div>
  );
}

export function safeMarkdownUrl(value: string): null {
  // Links are intentionally rendered as text in Task 5; keeping this guard
  // explicit prevents a later visual-link enhancement from accepting a risky URL.
  void value;
  return null;
}

type Block =
  | {
      readonly kind: 'code';
      readonly value: string;
      readonly truncated: boolean;
    }
  | { readonly kind: 'heading'; readonly value: string }
  | { readonly kind: 'list'; readonly value: string[] }
  | { readonly kind: 'paragraph'; readonly value: string };

function toBlocks(answer: string): readonly Block[] {
  const lines = answer.replace(/\r\n?/gu, '\n').split('\n');
  const blocks: Block[] = [];
  let code: string[] | null = null;
  for (let index = 0; index < lines.length; index += 1) {
    const line = lines[index];
    if (line.startsWith('```')) {
      if (code) {
        blocks.push(boundedCode(code.join('\n')));
        code = null;
      } else {
        code = [];
      }
      continue;
    }
    if (code) {
      code.push(line);
      continue;
    }
    if (/^#{1,6}\s+/u.test(line)) {
      blocks.push({ kind: 'heading', value: line.replace(/^#{1,6}\s+/u, '') });
    } else if (/^[-*+]\s+/u.test(line)) {
      const preceding = blocks.at(-1);
      const item = line.replace(/^[-*+]\s+/u, '');
      if (preceding?.kind === 'list') {
        preceding.value.push(item);
      } else {
        blocks.push({ kind: 'list', value: [item] });
      }
    } else if (line.trim()) {
      blocks.push({ kind: 'paragraph', value: line });
    }
  }
  if (code) blocks.push(boundedCode(code.join('\n')));
  return blocks;
}

function boundedCode(value: string): Block {
  const codePoints = [...value];
  const truncated = codePoints.length > MAX_CODE_BLOCK_CODE_POINTS;
  return {
    kind: 'code',
    value: codePoints.slice(0, MAX_CODE_BLOCK_CODE_POINTS).join(''),
    truncated,
  };
}

function renderBlock(block: Block) {
  switch (block.kind) {
    case 'heading':
      return <h3>{inlineText(block.value)}</h3>;
    case 'list':
      return (
        <ul>
          {block.value.map((item, index) => (
            <li key={index}>{inlineText(item)}</li>
          ))}
        </ul>
      );
    case 'code':
      return (
        <pre>
          <code>
            {block.value}
            {block.truncated ? '\n[code block truncated]' : ''}
          </code>
        </pre>
      );
    case 'paragraph':
      return <p>{inlineText(block.value)}</p>;
  }
}

function inlineText(value: string): string {
  // Keep incomplete `$` delimiters and any HTML exactly as literal, selectable text.
  return value.replace(/!\[[^\]]*\]\([^)]*\)/gu, '[image omitted]');
}
