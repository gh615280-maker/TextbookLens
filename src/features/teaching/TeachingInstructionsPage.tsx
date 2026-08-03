import { useLanguage } from '../../app/LanguageProvider';
import type { UiLanguage } from '../../lib/i18n';

const copy: Record<UiLanguage, { title: string; description: string }> = {
  'zh-CN': {
    title: '教学指令',
    description: '教学指令将在后续阶段实现。',
  },
  'zh-TW': {
    title: '教學指令',
    description: '教學指令將在後續階段實作。',
  },
  en: {
    title: 'Teaching instructions',
    description: 'Teaching instructions will be implemented in a later phase.',
  },
};

export function TeachingInstructionsPage() {
  const { uiLanguage } = useLanguage();
  const content = copy[uiLanguage];

  return (
    <section
      aria-labelledby="teaching-instructions-title"
      className="phase-page"
    >
      <h1 id="teaching-instructions-title">{content.title}</h1>
      <p>{content.description}</p>
    </section>
  );
}
