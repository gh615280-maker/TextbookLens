interface MarginMarkerButtonProps {
  id: string;
  kind: 'ai_conversation' | 'note';
  onActivate(annotationId: string): void;
}

export function MarginMarkerButton({
  id,
  kind,
  onActivate,
}: MarginMarkerButtonProps) {
  const ai = kind === 'ai_conversation';
  return (
    <button
      type="button"
      className={`margin-marker margin-marker-${ai ? 'ai' : 'note'}`}
      aria-label={ai ? '查看 AI 对话标记' : '查看个人批注'}
      data-marker-kind={kind}
      data-marker-shape={ai ? 'speech' : 'note'}
      data-marker-pattern={ai ? 'stripes' : 'dots'}
      onClick={() => onActivate(id)}
    >
      <span aria-hidden="true">{ai ? 'AI' : '◆'}</span>
    </button>
  );
}
