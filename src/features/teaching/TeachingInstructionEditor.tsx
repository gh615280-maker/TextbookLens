import { useMessage } from '../../app/LanguageProvider';

interface Props {
  draft: string;
  dirty: boolean;
  stage: 'loading' | 'saved' | 'saving' | 'conflict' | 'error';
  onChange(draft: string): void;
  onSave(): void;
  onClear(): void;
  onDefault(): void;
}

export function TeachingInstructionEditor({
  draft,
  dirty,
  stage,
  onChange,
  onSave,
  onClear,
  onDefault,
}: Props) {
  const message = useMessage();
  const scalarCount = Array.from(draft).length;
  const status =
    stage === 'saving'
      ? message('teaching.status.saving')
      : stage === 'conflict'
        ? message('teaching.status.conflict')
        : stage === 'error'
          ? message('teaching.status.error')
          : dirty
            ? message('teaching.status.unsaved')
            : message('teaching.status.saved');
  return (
    <section aria-labelledby="teaching-editor-title">
      <h2 id="teaching-editor-title">{message('teaching.editor.title')}</h2>
      <p aria-live="polite" role="status">
        {status}
      </p>
      <label htmlFor="teaching-instruction">
        {message('teaching.instructionLabel')}
      </label>
      <textarea
        id="teaching-instruction"
        maxLength={1000}
        onChange={(event) => onChange(event.target.value)}
        rows={16}
        value={draft}
      />
      <p>{scalarCount} / 1000</p>
      <button
        disabled={stage === 'saving' || !dirty}
        onClick={onSave}
        type="button"
      >
        {message('teaching.save')}
      </button>
      <button
        disabled={stage === 'saving' || !dirty}
        onClick={onClear}
        type="button"
      >
        {message('teaching.clear')}
      </button>
      <button
        disabled={stage === 'saving' || !dirty}
        onClick={onDefault}
        type="button"
      >
        {message('teaching.restoreDefault')}
      </button>
    </section>
  );
}
