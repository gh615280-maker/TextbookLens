interface ReaderToolbarProps {
  leftOpen: boolean;
  rightOpen: boolean;
  onToggleLeft(): void;
  onToggleRight(): void;
}

export function ReaderToolbar({ leftOpen, rightOpen, onToggleLeft, onToggleRight }: ReaderToolbarProps) {
  return <div className="reader-toolbar" role="toolbar" aria-label="阅读工具栏">
    <button type="button" aria-expanded={leftOpen} onClick={onToggleLeft}>{leftOpen ? '折叠目录' : '展开目录'}</button>
    <button type="button" aria-expanded={rightOpen} onClick={onToggleRight}>{rightOpen ? '折叠学习面板' : '展开学习面板'}</button>
  </div>;
}
