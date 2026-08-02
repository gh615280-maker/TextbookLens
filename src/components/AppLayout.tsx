import { Link, Outlet } from 'react-router-dom';

import { formatMessage } from '../lib/i18n';

export function AppLayout() {
  return (
    <div className="app-shell">
      <a className="skip-link" href="#main-content">
        {formatMessage('zh-CN', 'app.skipToContent')}
      </a>
      <header className="app-header">
        <p className="app-brand">TextbookLens</p>
        <nav aria-label={formatMessage('zh-CN', 'app.navigation')}>
          <ul className="app-navigation">
            <li>
              <Link to="/onboarding">
                {formatMessage('zh-CN', 'nav.onboarding')}
              </Link>
            </li>
            <li>
              <Link to="/library">{formatMessage('zh-CN', 'nav.library')}</Link>
            </li>
            <li>
              <Link to="/settings">
                {formatMessage('zh-CN', 'nav.settings')}
              </Link>
            </li>
          </ul>
        </nav>
      </header>
      <main id="main-content" tabIndex={-1}>
        <Outlet />
      </main>
    </div>
  );
}
