interface MarginMarkerButtonProps {
  id: string;
  kind: 'ai_conversation' | 'note';
  label?: string;
  onActivate(annotationId: string): void;
}

export function MarginMarkerButton({
  id,
  kind,
  label,
  onActivate,
}: MarginMarkerButtonProps) {
  const ai = kind === 'ai_conversation';
  return (
    <button
      type="button"
      className={`margin-marker margin-marker-${ai ? 'ai' : 'note'}`}
      aria-label={
        label ??
        (ai ? 'View AI conversation marker' : 'View personal note marker')
      }
      data-marker-kind={kind}
      data-marker-shape={ai ? 'speech' : 'note'}
      data-marker-pattern={ai ? 'stripes' : 'dots'}
      onClick={() => onActivate(id)}
    >
      <span className="margin-marker-dot" aria-hidden="true" />
      <span aria-hidden="true">{ai ? 'AI' : 'N'}</span>
    </button>
  );
}
