import type { MessageCatalog } from '../i18n';

export const en = {
  'nav.onboarding': 'Get started',
  'nav.library': 'Library',
  'nav.settings': 'Settings',
  'page.onboarding.title': 'Get started',
  'page.onboarding.description':
    'Choose a book, connect an AI service, and start reading.',
  'onboarding.book': 'Choose a book',
  'onboarding.provider': 'Connect an AI service',
  'onboarding.ready': 'Ready to read',
  'page.library.title': 'Library',
  'page.library.description':
    'Textbook import will be available in a later phase.',
  'page.reader.title': 'Read',
  'page.reader.description': 'The reader will be available in a later phase.',
  'page.overview.title': 'Learning overview',
  'page.overview.description':
    'Learning data will be available in a later phase.',
  'page.settings.title': 'Settings',
  'page.settings.description':
    'Application settings will be available in a later phase.',
  'app.skipToContent': 'Skip to main content',
  'app.navigation': 'Main navigation',
  'error.message':
    'The application interface encountered an error. Reopen the current page.',
  'error.retry': 'Retry',
  'loading.label': 'Loading',
  'language.menu': 'Application language',
  'language.zhCN': '简体中文',
  'language.zhTW': '繁體中文',
  'language.en': 'English',
  'language.switchFailed':
    'Unable to change the application language. Your previous language was restored.',
  'teaching.title': 'Teaching instructions',
  'teaching.description':
    'Set an editable teaching preference for future AI learning requests.',
  'teaching.loading': 'Loading teaching instructions',
  'teaching.conflict':
    'This instruction changed elsewhere. Reload the current version or preserve your draft before saving again.',
  'teaching.reload': 'Reload current',
  'teaching.copyDraft': 'Preserve my draft',
  'teaching.replaceDraft':
    'Replacing the current draft will discard its unsaved changes. Continue?',
  'teaching.replace': 'Replace draft',
  'teaching.keepEditing': 'Keep editing',
  'teaching.leaveWarning':
    'You have unsaved teaching-instruction changes. Leave this page?',
  'teaching.leave': 'Leave page',
  'teaching.stay': 'Stay',
  'example.greeting': 'Hello, {name}',
  'indexQuality.title': 'Index quality',
  'indexQuality.description':
    'Review local indexing results and manage the durable run.',
  'indexQuality.loading': 'Loading index status…',
  'indexQuality.retryLoading': 'Retry loading',
} satisfies MessageCatalog;
