import type { KeyboardEvent, PointerEvent } from 'react';

interface PanelTitleBarProps {
  readonly title: string;
  readonly status: string;
  readonly collapsed: boolean;
  readonly moveLabel: string;
  readonly collapseLabel: string;
  readonly expandLabel: string;
  readonly hideLabel: string;
  onDragStart(event: PointerEvent<HTMLDivElement>): void;
  onMoveKeyDown(event: KeyboardEvent<HTMLDivElement>): void;
  onHide(): void;
  onCollapse(): void;
}

/** Only this component initiates a panel move; content remains selectable. */
export function PanelTitleBar({
  title,
  status,
  collapsed,
  moveLabel,
  collapseLabel,
  expandLabel,
  hideLabel,
  onDragStart,
  onMoveKeyDown,
  onHide,
  onCollapse,
}: PanelTitleBarProps) {
  return (
    <header className="floating-panel-titlebar">
      <div
        aria-label={moveLabel}
        onPointerDown={onDragStart}
        onKeyDown={onMoveKeyDown}
        role="button"
        tabIndex={0}
      >
        <strong>{title}</strong>
        <span>{status}</span>
      </div>
      <button aria-expanded={!collapsed} onClick={onCollapse} type="button">
        {collapsed ? expandLabel : collapseLabel}
      </button>
      <button onClick={onHide} type="button">
        {hideLabel}
      </button>
    </header>
  );
}
