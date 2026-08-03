interface ReaderFirstHintProps {
  title: string;
  body: string;
  closeLabel: string;
  onComplete(): void;
}

export function ReaderFirstHint({
  title,
  body,
  closeLabel,
  onComplete,
}: ReaderFirstHintProps) {
  return (
    <aside
      className="reader-first-hint"
      aria-labelledby="reader-first-hint-title"
    >
      <div>
        <strong id="reader-first-hint-title">{title}</strong>
        <p>{body}</p>
      </div>
      <button type="button" onClick={onComplete}>
        {closeLabel}
      </button>
    </aside>
  );
}
