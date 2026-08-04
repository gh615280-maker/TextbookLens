import { useEffect, useRef } from 'react';

import { useMessage } from '../../app/LanguageProvider';

interface LibraryContextMenuProps {
  canOpen: boolean;
  canShowIndexStatus: boolean;
  canRemove: boolean;
  onClose(): void;
  onOpen(): void;
  onRemove(): void;
  onShowIndexStatus(): void;
}

export function LibraryContextMenu({
  canOpen,
  canShowIndexStatus,
  canRemove,
  onClose,
  onOpen,
  onRemove,
  onShowIndexStatus,
}: LibraryContextMenuProps) {
  const message = useMessage();
  const menuRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    menuRef.current?.focus();
  }, []);

  function activate(action: () => void) {
    onClose();
    action();
  }

  return (
    <div
      aria-label={message('library.contextMenu.label')}
      ref={menuRef}
      role="menu"
      tabIndex={-1}
      onKeyDown={(event) => {
        if (event.key === 'Escape') {
          event.preventDefault();
          onClose();
        }
      }}
    >
      <button
        disabled={!canOpen}
        role="menuitem"
        type="button"
        onClick={() => activate(onOpen)}
      >
        {message('library.contextMenu.open')}
      </button>
      <button
        disabled={!canShowIndexStatus}
        role="menuitem"
        type="button"
        onClick={() => activate(onShowIndexStatus)}
      >
        {message('library.contextMenu.indexStatus')}
      </button>
      {canRemove ? (
        <button
          role="menuitem"
          type="button"
          onClick={() => activate(onRemove)}
        >
          {message('library.contextMenu.remove')}
        </button>
      ) : null}
    </div>
  );
}
