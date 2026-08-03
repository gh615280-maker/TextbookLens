import { Link, NavLink } from 'react-router-dom';
import type { MouseEvent } from 'react';

import { useLanguage, useMessage } from '../app/LanguageProvider';
import type { UiLanguage } from '../lib/i18n';
import { LanguageMenu } from './LanguageMenu';

const navigationLabels: Record<
  UiLanguage,
  { library: string; teaching: string; aiServices: string; settings: string }
> = {
  'zh-CN': {
    library: '书库',
    teaching: '教学指令',
    aiServices: 'AI 服务',
    settings: '设置',
  },
  'zh-TW': {
    library: '圖書館',
    teaching: '教學指令',
    aiServices: 'AI 服務',
    settings: '設定',
  },
  en: {
    library: 'Library',
    teaching: 'Teaching instructions',
    aiServices: 'AI services',
    settings: 'Settings',
  },
};

export function TopNavigation() {
  const message = useMessage();
  const { uiLanguage } = useLanguage();
  const labels = navigationLabels[uiLanguage];

  function focusMainContent(event: MouseEvent<HTMLAnchorElement>) {
    event.preventDefault();
    document.getElementById('main-content')?.focus();
  }

  return (
    <>
      <a className="skip-link" href="#main-content" onClick={focusMainContent}>
        {message('app.skipToContent')}
      </a>
      <header className="app-header">
        <Link aria-label="TextbookLens" className="app-brand" to="/library">
          TextbookLens
        </Link>
        <nav aria-label={message('app.navigation')} className="top-navigation">
          <ul className="app-navigation">
            <li>
              <NavLink to="/library">{labels.library}</NavLink>
            </li>
            <li>
              <NavLink to="/teaching-instructions">{labels.teaching}</NavLink>
            </li>
            <li>
              <NavLink to="/ai-services">{labels.aiServices}</NavLink>
            </li>
          </ul>
        </nav>
        <div className="app-header__tools">
          <LanguageMenu />
          <NavLink to="/settings">{labels.settings}</NavLink>
        </div>
      </header>
    </>
  );
}
