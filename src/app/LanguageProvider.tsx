/* eslint-disable react-refresh/only-export-components */
import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from 'react';
import { invoke } from '@tauri-apps/api/core';

import type { AppSettingsDto } from '../lib/generated/settings';
import {
  formatMessage,
  type MessageKey,
  type MessageValues,
  type UiLanguage,
} from '../lib/i18n';
import { detectWindowsLanguage } from '../lib/locale';

interface LanguageSettingsApi {
  getAppSettings(): Promise<AppSettingsDto>;
  initializeUiLanguage(detected: UiLanguage): Promise<AppSettingsDto>;
  updateUiLanguage(language: UiLanguage): Promise<AppSettingsDto>;
}

class TauriLanguageSettingsApi implements LanguageSettingsApi {
  getAppSettings(): Promise<AppSettingsDto> {
    return invoke('get_app_settings');
  }

  initializeUiLanguage(detected: UiLanguage): Promise<AppSettingsDto> {
    return invoke('initialize_ui_language', { detected });
  }

  updateUiLanguage(language: UiLanguage): Promise<AppSettingsDto> {
    return invoke('update_ui_language', { language });
  }
}

const tauriLanguageSettingsApi = new TauriLanguageSettingsApi();

interface LanguageContextValue {
  uiLanguage: UiLanguage;
  isLoading: boolean;
  statusMessage: string | null;
  switchLanguage(language: UiLanguage): Promise<void>;
  message(key: MessageKey, values?: MessageValues): string;
}

export const LanguageContext = createContext<LanguageContextValue | null>(null);

interface LanguageProviderProps {
  children: ReactNode;
  api?: LanguageSettingsApi;
  detectedLanguages?: readonly string[];
}

export function LanguageProvider({
  children,
  api,
  detectedLanguages,
}: LanguageProviderProps) {
  const settingsApi = api ?? tauriLanguageSettingsApi;
  const systemLanguages = detectedLanguages ?? navigator.languages;
  const [uiLanguage, setUiLanguage] = useState<UiLanguage>('zh-CN');
  const [isLoading, setIsLoading] = useState(true);
  const [statusMessage, setStatusMessage] = useState<string | null>(null);

  useEffect(() => {
    let active = true;
    void settingsApi
      .getAppSettings()
      .then((settings) =>
        settings.uiLanguageInitialized
          ? settings
          : settingsApi.initializeUiLanguage(
              detectWindowsLanguage(systemLanguages),
            ),
      )
      .then((settings) => {
        if (!active) return;
        setUiLanguage(settings.uiLanguage);
      })
      .catch(() => {
        // Keep the initial UI language when SQLite is temporarily unavailable.
        // A later bootstrap can still obtain the persisted user choice.
      })
      .finally(() => {
        if (active) setIsLoading(false);
      });
    return () => {
      active = false;
    };
  }, [settingsApi, systemLanguages]);

  const switchLanguage = useCallback(
    async (language: UiLanguage) => {
      if (language === uiLanguage) return;
      const previousLanguage = uiLanguage;
      setStatusMessage(null);
      setUiLanguage(language);
      try {
        const settings = await settingsApi.updateUiLanguage(language);
        setUiLanguage(settings.uiLanguage);
      } catch {
        setUiLanguage(previousLanguage);
        setStatusMessage(
          formatMessage(previousLanguage, 'language.switchFailed'),
        );
      }
    },
    [settingsApi, uiLanguage],
  );

  const value = useMemo<LanguageContextValue>(
    () => ({
      uiLanguage,
      isLoading,
      statusMessage,
      switchLanguage,
      message: (key, values) => formatMessage(uiLanguage, key, values),
    }),
    [isLoading, statusMessage, switchLanguage, uiLanguage],
  );

  return (
    <LanguageContext.Provider value={value}>
      {children}
      <p aria-live="polite" role="status">
        {statusMessage}
      </p>
    </LanguageContext.Provider>
  );
}

export function useLanguage(): LanguageContextValue {
  const value = useContext(LanguageContext);
  if (!value) {
    throw new Error('useLanguage must be used inside LanguageProvider');
  }
  return value;
}

export function useMessage(): LanguageContextValue['message'] {
  return useLanguage().message;
}
