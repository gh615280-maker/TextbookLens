import type { UiLanguage } from '../../lib/i18n';
import type { ReaderSettings, ReaderTheme } from './api';

interface ReaderSettingsControlsProps {
  settings: ReaderSettings;
  format: 'pdf' | 'epub' | 'docx' | 'defaults';
  language?: UiLanguage;
  onChange(settings: ReaderSettings): void;
}

const copy = {
  'zh-CN': {
    theme: '主题',
    light: '浅色',
    dark: '深色',
    system: '跟随系统',
    font: '字体大小',
    line: '行距',
    width: '阅读宽度',
    zoom: 'PDF 缩放',
  },
  'zh-TW': {
    theme: '主題',
    light: '淺色',
    dark: '深色',
    system: '跟隨系統',
    font: '字體大小',
    line: '行距',
    width: '閱讀寬度',
    zoom: 'PDF 縮放',
  },
  en: {
    theme: 'Theme',
    light: 'Light',
    dark: 'Dark',
    system: 'Use system setting',
    font: 'Font size',
    line: 'Line spacing',
    width: 'Reading width',
    zoom: 'PDF zoom',
  },
} as const;

export function ReaderSettingsControls({
  settings,
  format,
  language = 'zh-CN',
  onChange,
}: ReaderSettingsControlsProps) {
  const labels = copy[language];
  const updateNumber = (field: keyof ReaderSettings, value: string) =>
    onChange({ ...settings, [field]: Number(value) });
  const updateTheme = (theme: ReaderTheme) => onChange({ ...settings, theme });

  return (
    <fieldset className="reader-settings">
      <label>
        {labels.theme}
        <select
          value={settings.theme}
          onChange={(event) => updateTheme(event.target.value as ReaderTheme)}
        >
          <option value="system">{labels.system}</option>
          <option value="light">{labels.light}</option>
          <option value="dark">{labels.dark}</option>
        </select>
      </label>
      {format !== 'pdf' && (
        <>
          <Range
            label={labels.font}
            value={settings.fontScale}
            min="0.75"
            max="2"
            step="0.05"
            onChange={(value) => updateNumber('fontScale', value)}
          />
          <Range
            label={labels.line}
            value={settings.lineHeight}
            min="1.2"
            max="2.4"
            step="0.1"
            onChange={(value) => updateNumber('lineHeight', value)}
          />
          <Range
            label={labels.width}
            value={settings.readerWidth}
            min="40"
            max="120"
            step="1"
            onChange={(value) => updateNumber('readerWidth', value)}
          />
        </>
      )}
      {(format === 'pdf' || format === 'defaults') && (
        <Range
          label={labels.zoom}
          value={settings.pdfZoom}
          min="0.5"
          max="3"
          step="0.1"
          onChange={(value) => updateNumber('pdfZoom', value)}
        />
      )}
    </fieldset>
  );
}

function Range({
  label,
  value,
  min,
  max,
  step,
  onChange,
}: {
  label: string;
  value: number;
  min: string;
  max: string;
  step: string;
  onChange(value: string): void;
}) {
  return (
    <label>
      {label}
      <input
        aria-label={label}
        type="range"
        min={min}
        max={max}
        step={step}
        value={value}
        onChange={(event) => onChange(event.target.value)}
      />
    </label>
  );
}
