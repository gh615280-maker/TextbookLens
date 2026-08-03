import { useEffect } from 'react';
import { Outlet, useLocation, useRouteError } from 'react-router-dom';

import { useLanguage } from '../app/LanguageProvider';
import type { UiLanguage } from '../lib/i18n';
import { TopNavigation } from './TopNavigation';

const routeTitles: Record<UiLanguage, Record<string, string>> = {
  'zh-CN': {
    '/library': '书库',
    '/teaching-instructions': '教学指令',
    '/ai-services': 'AI 服务',
    '/settings': '设置',
  },
  'zh-TW': {
    '/library': '圖書館',
    '/teaching-instructions': '教學指令',
    '/ai-services': 'AI 服務',
    '/settings': '設定',
  },
  en: {
    '/library': 'Library',
    '/teaching-instructions': 'Teaching instructions',
    '/ai-services': 'AI services',
    '/settings': 'Settings',
  },
};

const routeErrorCopy: Record<UiLanguage, { title: string; body: string }> = {
  'zh-CN': { title: '无法显示此页面', body: '请返回书库后重试。' },
  'zh-TW': { title: '無法顯示此頁面', body: '請返回圖書館後再試。' },
  en: {
    title: 'This page cannot be shown',
    body: 'Return to the library and try again.',
  },
};

function RouteTitle() {
  const { pathname } = useLocation();
  const { uiLanguage } = useLanguage();

  useEffect(() => {
    const title = routeTitles[uiLanguage][pathname];
    document.title = title ? `${title} · TextbookLens` : 'TextbookLens';
  }, [pathname, uiLanguage]);

  return null;
}

export function AppShell() {
  return (
    <div className="app-shell app-shell--standard">
      <RouteTitle />
      <TopNavigation />
      <main id="main-content" tabIndex={-1}>
        <Outlet />
      </main>
    </div>
  );
}

export function RouteErrorBoundary() {
  useRouteError();
  const { uiLanguage } = useLanguage();
  const copy = routeErrorCopy[uiLanguage];

  return (
    <main className="error-boundary" id="main-content" tabIndex={-1}>
      <h1>{copy.title}</h1>
      <p>{copy.body}</p>
    </main>
  );
}
