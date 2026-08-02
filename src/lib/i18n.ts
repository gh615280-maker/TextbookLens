import { zhCN } from './messages/zh-CN';

export type UiLanguage = 'zh-CN';
export type MessageKey = keyof typeof zhCN;
export type MessageValues = Record<string, string | number>;

const catalogs: Record<UiLanguage, Record<MessageKey, string>> = {
  'zh-CN': zhCN,
};

export function formatMessage(
  uiLanguage: UiLanguage,
  key: MessageKey,
  values: MessageValues = {},
): string {
  const message = catalogs[uiLanguage][key] ?? key;

  return message.replace(/\{(\w+)\}/g, (placeholder, name: string) => {
    const value = values[name];
    return value === undefined ? placeholder : String(value);
  });
}
