interface ReaderSidebarProps {
  title?: string;
  emptyMessage?: string;
}

export function ReaderSidebar({
  title = '目录',
  emptyMessage = '此教材没有可用目录。',
}: ReaderSidebarProps) {
  return (
    <nav className="reader-sidebar" aria-label={title}>
      <p>{emptyMessage}</p>
    </nav>
  );
}
