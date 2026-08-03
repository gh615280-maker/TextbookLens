import { useRef, useState, type FormEvent } from 'react';

import type { ReaderSearchHit } from './api';
import type { DocumentLocator } from '../../lib/generated/document';

interface ReaderSearchProps {
  bookId: string | null;
  format: 'pdf' | 'epub' | 'docx';
  search(
    bookId: string,
    query: string,
    limit: number,
  ): Promise<ReaderSearchHit[]>;
  onNavigate(locator: DocumentLocator): Promise<boolean> | boolean;
  labels?: {
    region: string;
    input: string;
    submit: string;
    loading: string;
    failed: string;
    empty: string;
    page(page: number): string;
    currentSection: string;
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
    empty: '没有搜索结果。',
    page: (page) => `第 ${page} 页`,
    currentSection: '当前章节',
  },
}: ReaderSearchProps) {
  const [query, setQuery] = useState('');
  const [results, setResults] = useState<ReaderSearchHit[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
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
      const next = await search(bookId, trimmed, 50);
      if (current === sequence.current) setResults(next);
    } catch {
      if (current === sequence.current) setError(labels.failed);
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
        <button type="submit">{labels.submit}</button>
      </form>
      {loading && <p role="status">{labels.loading}</p>}
      {error && <p role="alert">{error}</p>}
      {!loading && !error && query.trim() && results.length === 0 && (
        <p>{labels.empty}</p>
      )}
      <ol>
        {results.map((hit, index) => (
          <li key={`${index}-${hit.snippet}`}>
            <button
              type="button"
              onClick={() => {
                void onNavigate(hit.locator);
              }}
            >
              <span>{citation(hit, format, labels)}</span>
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
