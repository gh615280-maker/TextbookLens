import { describe, expect, it, vi } from 'vitest';

import {
  readSystemPreferences,
  subscribeSystemPreferences,
  systemPreferenceQueries,
  type MatchMedia,
} from './system-preferences';

function createMatchMedia(matches: Record<string, boolean>): MatchMedia {
  return (query) => ({ matches: matches[query] ?? false });
}

describe('system preferences', () => {
  it('reads color, forced-colors, and motion preferences without persistence', () => {
    expect(
      readSystemPreferences(
        createMatchMedia({
          [systemPreferenceQueries.dark]: true,
          [systemPreferenceQueries.forcedColors]: true,
          [systemPreferenceQueries.reducedMotion]: true,
        }),
      ),
    ).toEqual({ colorScheme: 'dark', forcedColors: true, reducedMotion: true });
  });

  it('subscribes and removes listeners for every supported preference', () => {
    const listeners = new Map<string, () => void>();
    const removeEventListener = vi.fn();
    const matchMedia: MatchMedia = (query) => ({
      matches: false,
      addEventListener: (_type, listener) => listeners.set(query, listener),
      removeEventListener,
    });
    const listener = vi.fn();

    const unsubscribe = subscribeSystemPreferences(listener, matchMedia);
    listeners.get(systemPreferenceQueries.dark)?.();
    unsubscribe();

    expect(listener).toHaveBeenCalledWith({
      colorScheme: 'light',
      forcedColors: false,
      reducedMotion: false,
    });
    expect(removeEventListener).toHaveBeenCalledTimes(3);
  });
});
