import {
  Button,
  Menu,
  MenuItem,
  MenuTrigger,
  Popover,
} from 'react-aria-components';

import { useLanguage } from '../app/LanguageProvider';
import type { UiLanguage } from '../lib/i18n';

const languages: readonly UiLanguage[] = ['zh-CN', 'zh-TW', 'en'];

export function LanguageMenu() {
  const { message, switchLanguage, uiLanguage } = useLanguage();
  const labels: Record<UiLanguage, string> = {
    'zh-CN': message('language.zhCN'),
    'zh-TW': message('language.zhTW'),
    en: message('language.en'),
  };

  return (
    <MenuTrigger>
      <Button aria-label={message('language.menu')}>
        {labels[uiLanguage]}
      </Button>
      <Popover>
        <Menu
          aria-label={message('language.menu')}
          selectedKeys={[uiLanguage]}
          selectionMode="single"
          onAction={(key) => void switchLanguage(key as UiLanguage)}
        >
          {languages.map((language) => (
            <MenuItem id={language} key={language}>
              {labels[language]}
            </MenuItem>
          ))}
        </Menu>
      </Popover>
    </MenuTrigger>
  );
}
