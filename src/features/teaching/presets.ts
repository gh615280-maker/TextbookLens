import type { UiLanguage } from '../../lib/i18n';

export interface TeachingPreset {
  id: string;
  label: Record<UiLanguage, string>;
  instruction: Record<UiLanguage, string>;
}

export const teachingPresets: readonly TeachingPreset[] = [
  {
    id: 'socratic',
    label: {
      'zh-CN': '苏格拉底式提问',
      'zh-TW': '蘇格拉底式提問',
      en: 'Socratic questioning',
    },
    instruction: {
      'zh-CN':
        '先用简短的问题帮助学习者检查自己的理解，再根据回答给出清晰的解释。',
      'zh-TW':
        '先用簡短的問題幫助學習者檢查自己的理解，再根據回答給出清晰的解釋。',
      en: 'Start with short questions that help the learner check their understanding, then give a clear explanation based on the response.',
    },
  },
  {
    id: 'examples-first',
    label: {
      'zh-CN': '先举例后定义',
      'zh-TW': '先舉例後定義',
      en: 'Examples before definitions',
    },
    instruction: {
      'zh-CN': '先给出一个简单、相关的例子，再说明定义和关键术语。',
      'zh-TW': '先給出一個簡單、相關的例子，再說明定義和關鍵術語。',
      en: 'Give a simple, relevant example first, then explain the definition and key terms.',
    },
  },
  {
    id: 'stepwise',
    label: {
      'zh-CN': '分步推导',
      'zh-TW': '分步推導',
      en: 'Stepwise derivation',
    },
    instruction: {
      'zh-CN': '用可核查的学生可见步骤说明推导，并为每一步给出简短理由。',
      'zh-TW': '用可核查的學生可見步驟說明推導，並為每一步給出簡短理由。',
      en: 'Explain derivations with checkable, student-visible steps and a brief reason for each step.',
    },
  },
  {
    id: 'stories',
    label: {
      'zh-CN': '故事与类比',
      'zh-TW': '故事與類比',
      en: 'Stories and analogies',
    },
    instruction: {
      'zh-CN': '必要时用简短故事或类比解释概念，并明确它只是帮助理解的比喻。',
      'zh-TW': '必要時用簡短故事或類比解釋概念，並明確它只是幫助理解的比喻。',
      en: 'When useful, use a short story or analogy to explain the concept and clearly mark it as an aid to understanding.',
    },
  },
  {
    id: 'conclusion-first',
    label: {
      'zh-CN': '先结论后证据',
      'zh-TW': '先結論後證據',
      en: 'Conclusion then evidence',
    },
    instruction: {
      'zh-CN': '先给出简明结论，再按重要性说明支持它的证据或理由。',
      'zh-TW': '先給出簡明結論，再按重要性說明支持它的證據或理由。',
      en: 'State a concise conclusion first, then explain the evidence or reasons that support it in order of importance.',
    },
  },
];
