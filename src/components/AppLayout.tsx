import { Link, Outlet } from 'react-router-dom';

import { useMessage } from '../app/LanguageProvider';
import { LanguageMenu } from './LanguageMenu';

export function AppLayout() {
  const message = useMessage();
  return (
    <div className="app-shell">
      <a className="skip-link" href="#main-content">
        {message('app.skipToContent')}
      </a>
      <header className="app-header">
        <p className="app-brand">TextbookLens</p>
        <nav aria-label={message('app.navigation')}>
          <ul className="app-navigation">
            <li>
              <Link to="/onboarding">{message('nav.onboarding')}</Link>
            </li>
            <li>
              <Link to="/library">{message('nav.library')}</Link>
            </li>
            <li>
              <Link to="/settings">{message('nav.settings')}</Link>
            </li>
          </ul>
        </nav>
        <LanguageMenu />
      </header>
      <main id="main-content" tabIndex={-1}>
        <Outlet />
      </main>
    </div>
  );
}
