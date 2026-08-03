interface ReaderPanelProps {
  content?: string;
}

export function ReaderPanel({ content }: ReaderPanelProps) {
  return content ? (
    <p className="reader-notice" role="status">
      {content}
    </p>
  ) : null;
}
