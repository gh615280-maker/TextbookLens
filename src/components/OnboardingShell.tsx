import { useEffect, type MouseEvent } from 'react';
import { Outlet } from 'react-router-dom';

import { useLanguage, useMessage } from '../app/LanguageProvider';
import { LanguageMenu } from './LanguageMenu';

export function OnboardingShell() {
  const message = useMessage();
  const { uiLanguage } = useLanguage();

  useEffect(() => {
    document.title =
      uiLanguage === 'en'
        ? 'Get started · TextbookLens'
        : uiLanguage === 'zh-TW'
          ? '開始使用 · TextbookLens'
          : '开始使用 · TextbookLens';
  }, [uiLanguage]);

  return (
    <div className="app-shell app-shell--onboarding">
      <a className="skip-link" href="#main-content" onClick={focusMainContent}>
        {message('app.skipToContent')}
      </a>
      <header className="app-header app-header--minimal">
        <p className="app-brand">TextbookLens</p>
        <LanguageMenu />
      </header>
      <main id="main-content" tabIndex={-1}>
        <Outlet />
      </main>
    </div>
  );
}
function focusMainContent(event: MouseEvent<HTMLAnchorElement>) {
  event.preventDefault();
  document.getElementById('main-content')?.focus();
}
