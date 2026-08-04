import type { MessageCatalog } from '../i18n';

export const zhTW = {
  'nav.onboarding': '開始使用',
  'nav.library': '圖書館',
  'nav.settings': '設定',
  'page.onboarding.title': '開始使用',
  'page.onboarding.description': '選擇教材、連接 AI 服務，然後開始閱讀。',
  'onboarding.book': '選擇教材',
  'onboarding.provider': '連接 AI 服務',
  'onboarding.ready': '準備閱讀',
  'page.library.title': '圖書館',
  'page.library.description': '匯入教材功能將在後續階段提供。',
  'page.reader.title': '閱讀',
  'page.reader.description': '閱讀器將在後續階段提供。',
  'page.overview.title': '學習總覽',
  'page.overview.description': '學習資料將在後續階段提供。',
  'page.settings.title': '設定',
  'page.settings.description': '應用程式設定將在後續階段提供。',
  'app.skipToContent': '跳至主要內容',
  'app.navigation': '主導覽',
  'error.message': '應用程式介面發生錯誤。請重新開啟目前頁面。',
  'error.retry': '重試',
  'loading.label': '正在載入',
  'language.menu': '應用程式語言',
  'language.zhCN': '简体中文',
  'language.zhTW': '繁體中文',
  'language.en': 'English',
  'language.switchFailed': '無法切換應用程式語言。已還原原來的語言。',
  'teaching.title': '教學指令',
  'teaching.description': '為之後的 AI 學習請求設定可編輯的教學偏好。',
  'teaching.loading': '正在載入教學指令',
  'teaching.conflict':
    '此指令已在其他位置修改。請重新載入目前版本，或先保留草稿再儲存。',
  'teaching.reload': '重新載入目前版本',
  'teaching.copyDraft': '保留我的草稿',
  'teaching.replaceDraft': '替換目前草稿會捨棄未儲存的變更。要繼續嗎？',
  'teaching.replace': '替換草稿',
  'teaching.keepEditing': '繼續編輯',
  'teaching.leaveWarning': '教學指令有未儲存的變更。要離開此頁面嗎？',
  'teaching.leave': '離開頁面',
  'teaching.stay': '留在此頁',
  'example.greeting': '你好，{name}',
  'indexStart.action': 'AI-assisted index',
  'indexStart.title': 'AI-assisted indexing',
  'indexStart.description':
    'Check this PDF locally and send only pages with unreliable text after confirmation.',
  'indexStart.preparing': 'Checking the local PDF and vision profile…',
  'indexStart.confirmTitle': 'Start AI-assisted indexing?',
  'indexStart.details': 'Profile: {profile}. Model: {model}. Pages: {pages}.',
  'indexStart.sent':
    'Local page images for these {pages} abnormal pages will be sent for structured page analysis. This may incur provider charges.',
  'indexStart.noPrompt':
    'Do not show this index-start prompt again for this profile',
  'indexStart.reject': 'Continue without AI indexing',
  'indexStart.confirm': 'Confirm and start',
  'indexStart.starting': 'Starting index run…',
  'indexStart.startFailed':
    'The index run could not be started. Review the profile and try again.',
  'indexStart.notReadyPdf':
    'AI-assisted indexing is available only for ready PDF books. This book can still be opened locally.',
  'indexStart.reliableOnly':
    'This PDF already has reliable local text. AI-assisted page indexing is not needed.',
  'indexStart.profileMissing':
    'Choose a verified vision profile with an available credential before starting AI-assisted indexing.',
  'indexStart.localFailure':
    'The PDF could not be checked locally. No page was sent. You can continue local reading or try again later.',
  'indexStart.configure': 'Configure AI services',
  'indexStart.continueReading': 'Continue local reading',
  'indexStart.localLimitation':
    'This PDF has unreliable or no local text. Search and text-based AI are limited; local page reading remains available.',
  'indexQuality.title': 'Index quality',
  'indexQuality.description':
    'Review local indexing results and manage the durable run.',
  'indexQuality.loading': 'Loading index status…',
  'indexQuality.retryLoading': 'Retry loading',
} satisfies MessageCatalog;
