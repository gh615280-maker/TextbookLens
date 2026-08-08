import { useMessage } from '../../app/LanguageProvider';
import type { ImportEvent, ImportStage } from './parser-contract';

interface ImportProgressProps {
  event: ImportEvent;
  onCancel(): void;
}

const stages: readonly ImportStage[] = ['copying', 'parsing', 'indexing'];

export function ImportProgress({ event, onCancel }: ImportProgressProps) {
  const message = useMessage();
  const maximum = event.total > 0 ? event.total : 1;
  return (
    <section
      aria-atomic="true"
      aria-labelledby="import-progress-title"
      aria-live="polite"
      className="import-progress"
      role="status"
    >
      <h2 id="import-progress-title">{message('import.title')}</h2>
      <ol className="import-stages">
        {stages.map((stage) => (
          <li
            key={stage}
            aria-current={stage === event.stage ? 'step' : undefined}
          >
            {message(`import.stage.${stage}`)}
          </li>
        ))}
      </ol>
      <progress
        aria-label={message('import.progress')}
        max={maximum}
        value={Math.min(event.completed, maximum)}
      />
      <p>
        {event.total > 0
          ? `${Math.min(event.completed, event.total)} / ${event.total}`
          : message('import.preparing')}
      </p>
      <button type="button" onClick={onCancel}>
        {message('import.cancel')}
      </button>
    </section>
  );
}
