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
  const scalarCount = Array.from(draft).length;
  const status =
    stage === 'saving'
      ? 'Saving'
      : stage === 'conflict'
        ? 'Conflict'
        : stage === 'error'
          ? 'Error'
          : dirty
            ? 'Unsaved changes'
            : 'Saved';
  return (
    <section aria-labelledby="teaching-editor-title">
      <h2 id="teaching-editor-title">Teaching instruction</h2>
      <p aria-live="polite" role="status">
        {status}
      </p>
      <label htmlFor="teaching-instruction">Instruction</label>
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
        Save
      </button>
      <button
        disabled={stage === 'saving' || !dirty}
        onClick={onClear}
        type="button"
      >
        Clear draft
      </button>
      <button
        disabled={stage === 'saving' || !dirty}
        onClick={onDefault}
        type="button"
      >
        Restore default
      </button>
    </section>
  );
}
