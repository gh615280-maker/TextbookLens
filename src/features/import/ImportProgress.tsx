import type { ImportEvent, ImportStage } from './parser-contract';

interface ImportProgressProps {
  event: ImportEvent;
  onCancel(): void;
}

const stages: Array<{ id: ImportStage; label: string }> = [
  { id: 'copying', label: '复制文件' },
  { id: 'parsing', label: '解析内容' },
  { id: 'indexing', label: '建立索引' },
];

export function ImportProgress({ event, onCancel }: ImportProgressProps) {
  const maximum = event.total > 0 ? event.total : 1;
  return (
    <section
      aria-atomic="true"
      aria-labelledby="import-progress-title"
      aria-live="polite"
      className="import-progress"
      role="status"
    >
      <h2 id="import-progress-title">正在导入教材</h2>
      <ol className="import-stages">
        {stages.map((stage) => (
          <li
            key={stage.id}
            aria-current={stage.id === event.stage ? 'step' : undefined}
          >
            {stage.label}
          </li>
        ))}
      </ol>
      <progress
        aria-label="导入进度"
        max={maximum}
        value={Math.min(event.completed, maximum)}
      />
      <p>
        {event.total > 0
          ? `${Math.min(event.completed, event.total)} / ${event.total}`
          : '正在准备'}
      </p>
      <button type="button" onClick={onCancel}>
        取消导入
      </button>
    </section>
  );
}
