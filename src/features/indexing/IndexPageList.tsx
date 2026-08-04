import type { IndexPageReviewDto } from '../../lib/generated/indexing';

export function IndexPageList({
  pages,
  selectedPageId,
  onSelect,
}: {
  pages: IndexPageReviewDto[];
  selectedPageId: string | null;
  onSelect(pageId: string): void;
}) {
  const reviewPages = pages.filter(
    (page) => page.status === 'needs_review' || page.status === 'failed',
  );
  return (
    <section aria-labelledby="index-page-list-title">
      <h2 id="index-page-list-title">Pages needing attention</h2>
      {reviewPages.length === 0 ? <p>No pages need review.</p> : null}
      <ol aria-label="Pages needing attention">
        {reviewPages.map((page) => (
          <li key={page.id}>
            <button
              aria-current={selectedPageId === page.id ? 'page' : undefined}
              type="button"
              onClick={() => onSelect(page.id)}
            >
              Page {page.pageNumber}: {page.status.replaceAll('_', ' ')}
            </button>
          </li>
        ))}
      </ol>
    </section>
  );
}
