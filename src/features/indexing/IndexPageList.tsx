import { useMessage } from '../../app/LanguageProvider';
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
  const message = useMessage();
  const reviewPages = pages.filter(
    (page) => page.status === 'needs_review' || page.status === 'failed',
  );
  return (
    <section aria-labelledby="index-page-list-title">
      <h2 id="index-page-list-title">{message('indexQuality.pages.title')}</h2>
      {reviewPages.length === 0 ? (
        <p>{message('indexQuality.pages.empty')}</p>
      ) : null}
      <ol aria-label={message('indexQuality.pages.title')}>
        {reviewPages.map((page) => (
          <li key={page.id}>
            <button
              aria-current={selectedPageId === page.id ? 'page' : undefined}
              type="button"
              onClick={() => onSelect(page.id)}
            >
              {message('indexQuality.pages.item', {
                page: page.pageNumber,
                status: message(`indexQuality.status.${page.status}`),
              })}
            </button>
          </li>
        ))}
      </ol>
    </section>
  );
}
