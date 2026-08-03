import { useEffect } from 'react';
import { Outlet, useParams } from 'react-router-dom';

import { useLanguage } from '../app/LanguageProvider';

export function ReaderShell() {
  const { bookId } = useParams();
  const { uiLanguage } = useLanguage();

  useEffect(() => {
    const title =
      uiLanguage === 'en' ? 'Read' : uiLanguage === 'zh-TW' ? '閱讀' : '阅读';
    document.title = `${title}${bookId ? ` · ${bookId}` : ''} · TextbookLens`;
  }, [bookId, uiLanguage]);

  return (
    <div className="app-shell app-shell--reader">
      <Outlet />
    </div>
  );
}
