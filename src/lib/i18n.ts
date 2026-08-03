import { en } from './messages/en';
import { zhCN } from './messages/zh-CN';
import { zhTW } from './messages/zh-TW';

export type UiLanguage = 'zh-CN' | 'zh-TW' | 'en';
export type MessageKey = keyof typeof zhCN;
export type MessageValues = Record<string, string | number>;
export type MessageCatalog = Record<MessageKey, string>;

const catalogs = {
  'zh-CN': zhCN,
  'zh-TW': zhTW,
  en,
} satisfies Record<UiLanguage, MessageCatalog>;

export function formatMessage(
  uiLanguage: UiLanguage,
  key: MessageKey,
  values: MessageValues = {},
): string {
  const message = catalogs[uiLanguage][key];
  if (typeof message !== 'string' || message.trim().length === 0) {
    throw new Error(`Missing message: ${key}`);
  }

  return message.replace(/\{(\w+)\}/g, (_placeholder, name: string) => {
    const value = values[name];
    if (value === undefined) {
      throw new Error(`Missing value "${name}" for message: ${key}`);
    }
    return String(value);
  });
}

export function messageCatalogs(): Record<UiLanguage, MessageCatalog> {
  return catalogs;
}
