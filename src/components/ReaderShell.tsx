import { useEffect } from 'react';
import { Outlet, useParams } from 'react-router-dom';

import { useLanguage, useMessage } from '../app/LanguageProvider';

export function ReaderShell() {
  const { bookId } = useParams();
  const { uiLanguage } = useLanguage();
  const message = useMessage();

  useEffect(() => {
    const title =
      uiLanguage === 'en' ? 'Read' : uiLanguage === 'zh-TW' ? '閱讀' : '阅读';
    document.title = `${title}${bookId ? ` · ${bookId}` : ''} · TextbookLens`;
  }, [bookId, uiLanguage]);

  return (
    <div className="app-shell app-shell--reader">
      <a
        className="skip-link"
        href="#reader-main-content"
        onClick={() => {
          window.setTimeout(() =>
            document.getElementById('reader-main-content')?.focus(),
          );
        }}
      >
        {message('app.skipToContent')}
      </a>
      <Outlet />
    </div>
  );
}
