import type { ReaderSettings } from './api';

interface ReaderSettingsControlsProps {
  settings: ReaderSettings;
  onChange(settings: ReaderSettings): void;
}

export function ReaderSettingsControls({
  settings,
  onChange,
}: ReaderSettingsControlsProps) {
  const update = (field: keyof ReaderSettings, value: string) =>
    onChange({ ...settings, [field]: Number(value) });
  return (
    <fieldset className="reader-settings" aria-label="阅读设置">
      <label>
        字体大小
        <input
          aria-label="字体大小"
          type="range"
          min="0.75"
          max="2"
          step="0.05"
          value={settings.fontScale}
          onChange={(event) => update('fontScale', event.target.value)}
        />
      </label>
      <label>
        行距
        <input
          aria-label="行距"
          type="range"
          min="1.2"
          max="2.4"
          step="0.1"
          value={settings.lineHeight}
          onChange={(event) => update('lineHeight', event.target.value)}
        />
      </label>
      <label>
        阅读宽度
        <input
          aria-label="阅读宽度"
          type="range"
          min="40"
          max="120"
          step="1"
          value={settings.readerWidth}
          onChange={(event) => update('readerWidth', event.target.value)}
        />
      </label>
      <label>
        PDF 缩放
        <input
          aria-label="PDF 缩放"
          type="range"
          min="0.5"
          max="3"
          step="0.1"
          value={settings.pdfZoom}
          onChange={(event) => update('pdfZoom', event.target.value)}
        />
      </label>
    </fieldset>
  );
}
