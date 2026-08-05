import type { PointerEvent } from 'react';

interface PanelTitleBarProps {
  readonly title: string;
  readonly status: string;
  readonly collapsed: boolean;
  onDragStart(event: PointerEvent<HTMLDivElement>): void;
  onHide(): void;
  onCollapse(): void;
}

/** Only this component initiates a panel move; content remains selectable. */
export function PanelTitleBar({
  title,
  status,
  collapsed,
  onDragStart,
  onHide,
  onCollapse,
}: PanelTitleBarProps) {
  return (
    <header className="floating-panel-titlebar">
      <div
        aria-label="Move learning panel"
        onPointerDown={onDragStart}
        role="presentation"
      >
        <strong>{title}</strong>
        <span>{status}</span>
      </div>
      <button aria-expanded={!collapsed} onClick={onCollapse} type="button">
        {collapsed ? 'Expand' : 'Collapse'}
      </button>
      <button onClick={onHide} type="button">
        Hide
      </button>
    </header>
  );
}
