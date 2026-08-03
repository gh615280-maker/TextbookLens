import { useLanguage } from '../../app/LanguageProvider';
import { teachingPresets } from './presets';

export function TeachingPresetMenu({
  disabled,
  onSelect,
}: {
  disabled: boolean;
  onSelect(instruction: string): void;
}) {
  const { uiLanguage } = useLanguage();
  return (
    <section aria-labelledby="teaching-presets-title">
      <h2 id="teaching-presets-title">Presets</h2>
      {teachingPresets.map((preset) => (
        <button
          disabled={disabled}
          key={preset.id}
          onClick={() => onSelect(preset.instruction[uiLanguage])}
          type="button"
        >
          {preset.label[uiLanguage]}
        </button>
      ))}
    </section>
  );
}
