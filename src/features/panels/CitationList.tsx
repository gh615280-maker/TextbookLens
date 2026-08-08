import { useMessage } from '../../app/LanguageProvider';
import type { Citation } from '../../lib/generated/conversation';

interface CitationListProps {
  readonly citations: readonly Pick<Citation, 'id' | 'label' | 'quoteable'>[];
  readonly emptyLabel: string;
}

/** Citation navigation is intentionally deferred to Task 6's verified reader locators. */
export function CitationList({ citations, emptyLabel }: CitationListProps) {
  const message = useMessage();
  if (!citations.length) return <p className="citation-empty">{emptyLabel}</p>;
  return (
    <ol aria-label={message('panel.citationList')} className="citation-list">
      {citations.map((citation) => (
        <li key={citation.id}>
          <span>{citation.label}</span>
          {!citation.quoteable ? (
            <span> ({message('panel.notQuoteable')})</span>
          ) : null}
        </li>
      ))}
    </ol>
  );
}
