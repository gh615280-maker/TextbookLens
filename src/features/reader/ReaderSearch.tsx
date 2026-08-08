import { useRef, useState, type FormEvent } from 'react';

import type { DocumentLocator } from '../../lib/generated/document';
import type { ReaderSearchHit, ReaderSearchScope } from './api';

interface ReaderSearchProps {
  bookId: string | null;
  format: 'pdf' | 'epub' | 'docx';
  search(
    bookId: string,
    query: string,
    limit: number,
    scope: ReaderSearchScope,
  ): Promise<ReaderSearchHit[]>;
  onNavigate(locator: DocumentLocator): Promise<boolean> | boolean;
  labels?: {
    region: string;
    input: string;
    submit: string;
    loading: string;
    failed: string;
    navigateFailed: string;
    empty: string;
    page(page: number): string;
    currentSection: string;
    scope: string;
    all: string;
    book: string;
    question: string;
  };
}

export function ReaderSearch({
  bookId,
  format,
  search,
  onNavigate,
  labels = {
    region: '书内搜索',
    input: '搜索书内内容',
    submit: '搜索',
    loading: '正在搜索…',
    failed: '搜索失败，请重试。',
    navigateFailed: '无法精确恢复此结果的原文位置。',
    empty: '没有搜索结果。',
    page: (page) => `第 ${page} 页`,
    currentSection: '当前章节',
    scope: '搜索范围',
    all: '全部',
    book: '仅原文',
    question: '仅问题简述',
  },
}: ReaderSearchProps) {
  const [query, setQuery] = useState('');
  const [scope, setScope] = useState<ReaderSearchScope>('all');
  const [results, setResults] = useState<ReaderSearchHit[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<'search' | 'navigate' | null>(null);
  const sequence = useRef(0);
  const submit = async (event: FormEvent) => {
    event.preventDefault();
    const trimmed = query.trim();
    if (!trimmed || !bookId) {
      setResults([]);
      return;
    }
    const current = ++sequence.current;
    setLoading(true);
    setError(null);
    try {
      const next = await search(bookId, trimmed, 50, scope);
      if (current === sequence.current) setResults(next);
    } catch {
      if (current === sequence.current) setError('search');
    } finally {
      if (current === sequence.current) setLoading(false);
    }
  };
  return (
    <section aria-label={labels.region}>
      <form
        onSubmit={(event) => {
          void submit(event);
        }}
      >
        <label>
          {labels.input}
          <input
            value={query}
            onChange={(event) => setQuery(event.target.value)}
          />
        </label>
        <fieldset className="reader-search__scope">
          <legend>{labels.scope}</legend>
          {(
            [
              ['all', labels.all],
              ['book', labels.book],
              ['question', labels.question],
            ] as const
          ).map(([value, label]) => (
            <label key={value}>
              <input
                type="radio"
                name="reader-search-scope"
                value={value}
                checked={scope === value}
                onChange={() => setScope(value)}
              />
              {label}
            </label>
          ))}
        </fieldset>
        <button type="submit">{labels.submit}</button>
      </form>
      {loading && <p role="status">{labels.loading}</p>}
      {error && (
        <p role="alert">
          {error === 'search' ? labels.failed : labels.navigateFailed}
        </p>
      )}
      {!loading && !error && query.trim() && results.length === 0 && (
        <p>{labels.empty}</p>
      )}
      <ol>
        {results.map((hit, index) => (
          <li key={`${index}-${hit.snippet}`}>
            <button
              type="button"
              onClick={() => {
                setError(null);
                void Promise.resolve(onNavigate(hit.locator))
                  .then((restored) => {
                    if (!restored) setError('navigate');
                  })
                  .catch(() => setError('navigate'));
              }}
            >
              <span>
                {hit.source === 'question'
                  ? labels.question
                  : citation(hit, format, labels)}
              </span>
              <span>{hit.snippet}</span>
            </button>
          </li>
        ))}
      </ol>
    </section>
  );
}

function citation(
  hit: ReaderSearchHit,
  format: ReaderSearchProps['format'],
  labels: NonNullable<ReaderSearchProps['labels']>,
): string {
  return format === 'pdf' && hit.locator.format === 'pdf'
    ? labels.page(hit.locator.startPage)
    : (hit.sectionTitle ?? labels.currentSection);
}
