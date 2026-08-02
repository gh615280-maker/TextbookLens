import { useMemo, useState, type ReactNode } from 'react';

import type { ReaderSection } from './api';
import type { DocumentLocator } from '../../lib/generated/document';

interface ReaderTocProps {
  sections: readonly ReaderSection[];
  currentSectionId?: string | null;
  onNavigate(locator: DocumentLocator): Promise<boolean> | boolean;
}

export function ReaderToc({
  sections,
  currentSectionId = null,
  onNavigate,
}: ReaderTocProps) {
  const [expanded, setExpanded] = useState(
    () =>
      new Set(
        sections
          .filter((section) => section.parentId === null)
          .map((section) => section.id),
      ),
  );
  const children = useMemo(() => {
    const result = new Map<string | null, ReaderSection[]>();
    for (const section of [...sections].sort((a, b) => a.ordinal - b.ordinal))
      result.set(section.parentId, [
        ...(result.get(section.parentId) ?? []),
        section,
      ]);
    return result;
  }, [sections]);
  const render = (parentId: string | null, level: number): ReactNode =>
    (children.get(parentId) ?? []).map((section) => {
      const nested = children.get(section.id) ?? [];
      const isExpanded = expanded.has(section.id);
      return (
        <li key={section.id}>
          <div>
            {nested.length > 0 && (
              <button
                type="button"
                aria-label={`${isExpanded ? 'Collapse' : 'Expand'} ${section.title}`}
                aria-expanded={isExpanded}
                onClick={() =>
                  setExpanded((value) => {
                    const next = new Set(value);
                    if (next.has(section.id)) next.delete(section.id);
                    else next.add(section.id);
                    return next;
                  })
                }
              >
                {isExpanded ? '−' : '+'}
              </button>
            )}
            <button
              type="button"
              aria-current={
                section.id === currentSectionId ? 'page' : undefined
              }
              style={{ marginInlineStart: `${level * 12}px` }}
              onClick={() => {
                void onNavigate(section.locator);
              }}
            >
              {section.title}
            </button>
          </div>
          {nested.length > 0 && isExpanded && (
            <ul>{render(section.id, level + 1)}</ul>
          )}
        </li>
      );
    });
  return (
    <aside className="reader-sidebar" aria-label="目录">
      <h2>目录</h2>
      <nav aria-label="教材目录">
        <ul>{render(null, 0)}</ul>
      </nav>
    </aside>
  );
}
