import { useRef, useState, type FormEvent } from 'react';

import type { ReaderSearchHit } from './api';
import type { DocumentLocator } from '../../lib/generated/document';

interface ReaderSearchProps { bookId: string | null; format: 'pdf' | 'epub' | 'docx'; search(bookId: string, query: string, limit: number): Promise<ReaderSearchHit[]>; onNavigate(locator: DocumentLocator): Promise<boolean> | boolean; }
export function ReaderSearch({ bookId, format, search, onNavigate }: ReaderSearchProps) {
  const [query, setQuery] = useState(''); const [results, setResults] = useState<ReaderSearchHit[]>([]); const [loading, setLoading] = useState(false); const [error, setError] = useState<string | null>(null); const sequence = useRef(0);
  const submit = async (event: FormEvent) => { event.preventDefault(); const trimmed = query.trim(); if (!trimmed || !bookId) { setResults([]); return; } const current = ++sequence.current; setLoading(true); setError(null); try { const next = await search(bookId, trimmed, 50); if (current === sequence.current) setResults(next); } catch { if (current === sequence.current) setError('搜索失败，请重试。'); } finally { if (current === sequence.current) setLoading(false); } };
  return <section aria-label="书内搜索"><form onSubmit={(event) => { void submit(event); }}><label>搜索书内内容<input value={query} onChange={(event) => setQuery(event.target.value)} /></label><button type="submit">搜索</button></form>{loading && <p role="status">正在搜索…</p>}{error && <p role="alert">{error}</p>}{!loading && !error && query.trim() && results.length === 0 && <p>没有搜索结果。</p>}<ol>{results.map((hit, index) => <li key={`${index}-${hit.snippet}`}><button type="button" onClick={() => { void onNavigate(hit.locator); }}><span>{citation(hit, format)}</span><span>{hit.snippet}</span></button></li>)}</ol></section>;
}
function citation(hit: ReaderSearchHit, format: ReaderSearchProps['format']): string { return format === 'pdf' && hit.locator.format === 'pdf' ? `第 ${hit.locator.startPage} 页` : hit.sectionTitle ?? '当前章节'; }
