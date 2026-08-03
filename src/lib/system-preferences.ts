export interface SystemPreferences {
  colorScheme: 'light' | 'dark';
  forcedColors: boolean;
  reducedMotion: boolean;
}

export interface MediaQueryListLike {
  matches: boolean;
  addEventListener?(type: 'change', listener: () => void): void;
  removeEventListener?(type: 'change', listener: () => void): void;
}

export type MatchMedia = (query: string) => MediaQueryListLike;

export const systemPreferenceQueries = {
  dark: '(prefers-color-scheme: dark)',
  forcedColors: '(forced-colors: active)',
  reducedMotion: '(prefers-reduced-motion: reduce)',
} as const;

export function readSystemPreferences(
  matchMedia: MatchMedia = window.matchMedia.bind(window),
): SystemPreferences {
  return {
    colorScheme: matchMedia(systemPreferenceQueries.dark).matches
      ? 'dark'
      : 'light',
    forcedColors: matchMedia(systemPreferenceQueries.forcedColors).matches,
    reducedMotion: matchMedia(systemPreferenceQueries.reducedMotion).matches,
  };
}

export function subscribeSystemPreferences(
  listener: (preferences: SystemPreferences) => void,
  matchMedia: MatchMedia = window.matchMedia.bind(window),
): () => void {
  const queries = Object.values(systemPreferenceQueries).map(matchMedia);
  const notify = () => listener(readSystemPreferences(matchMedia));

  queries.forEach((query) => query.addEventListener?.('change', notify));
  return () => {
    queries.forEach((query) => query.removeEventListener?.('change', notify));
  };
}
