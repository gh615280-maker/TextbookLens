import type { PreparationSummary } from './learning-contract';

export interface LearningConfirmationLabels {
  title: string;
  details: string;
  noPrompt: string;
  cancel: string;
  continue: string;
  imageRisk: string;
  costRisk: string;
}

interface LearningConfirmationDialogProps {
  summary: Readonly<PreparationSummary>;
  labels: LearningConfirmationLabels;
  busy?: boolean;
  onCancel(): void;
  onConfirm(skipPrompt: boolean): void;
}

export function LearningConfirmationDialog({
  summary,
  labels,
  busy = false,
  onCancel,
  onConfirm,
}: LearningConfirmationDialogProps) {
  return (
    <div className="learning-confirmation-backdrop" role="presentation">
      <section
        aria-label={labels.title}
        aria-modal="true"
        className="learning-confirmation-dialog"
        role="dialog"
      >
        <h2>{labels.title}</h2>
        <p>
          {labels.details
            .replace('{provider}', summary.providerDisplayName)
            .replace('{profile}', summary.profileDisplayName)
            .replace('{model}', summary.modelDisplayName)
            .replace('{tokens}', String(summary.estimatedInputTokens))
            .replace('{sources}', String(summary.sourceCount))
            .replace('{citations}', String(summary.citationCount))}
        </p>
        <p>{summary.willSendImage ? labels.imageRisk : labels.costRisk}</p>
        <label>
          <input
            data-testid="learning-no-prompt"
            type="checkbox"
            disabled={busy}
          />
          {labels.noPrompt}
        </label>
        <footer>
          <button type="button" disabled={busy} onClick={onCancel}>
            {labels.cancel}
          </button>
          <button
            type="button"
            disabled={busy}
            onClick={(event) =>
              onConfirm(
                event.currentTarget
                  .closest('[role=dialog]')
                  ?.querySelector<HTMLInputElement>(
                    '[data-testid=learning-no-prompt]',
                  )?.checked ?? false,
              )
            }
          >
            {labels.continue}
          </button>
        </footer>
      </section>
    </div>
  );
}
