interface ReaderPanelProps { content?: string; }

export function ReaderPanel({ content = '学习内容将在后续阶段显示。' }: ReaderPanelProps) {
  return <aside className="reader-panel" aria-label="学习面板" aria-live="polite"><h2>学习面板</h2><p>{content}</p></aside>;
}
