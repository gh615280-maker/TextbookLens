import { useLanguage } from '../../app/LanguageProvider';
import type { UiLanguage } from '../../lib/i18n';

const copy: Record<UiLanguage, { title: string; description: string }> = {
  'zh-CN': {
    title: 'AI 服务',
    description: 'AI 服务将在后续阶段实现。',
  },
  'zh-TW': {
    title: 'AI 服務',
    description: 'AI 服務將在後續階段實作。',
  },
  en: {
    title: 'AI services',
    description: 'AI services will be implemented in a later phase.',
  },
};

export function AiServicesPage() {
  const { uiLanguage } = useLanguage();
  const content = copy[uiLanguage];

  return (
    <section aria-labelledby="ai-services-title" className="phase-page">
      <h1 id="ai-services-title">{content.title}</h1>
      <p>{content.description}</p>
    </section>
  );
}
