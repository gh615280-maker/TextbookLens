CREATE TABLE app_settings_replanned (
  id INTEGER PRIMARY KEY NOT NULL CHECK (id = 1),
  onboarding_completed INTEGER NOT NULL DEFAULT 0 CHECK (onboarding_completed IN (0, 1)),
  active_provider_profile_id TEXT REFERENCES provider_profiles(id) ON DELETE SET NULL,
  theme TEXT NOT NULL DEFAULT 'system' CHECK (theme IN ('light', 'dark', 'system')),
  context_mode TEXT NOT NULL DEFAULT 'standard' CHECK (context_mode IN ('standard', 'long')),
  ui_language TEXT NOT NULL DEFAULT 'en' CHECK (ui_language IN ('zh-CN', 'zh-TW', 'en')),
  ui_language_initialized INTEGER NOT NULL DEFAULT 0 CHECK (ui_language_initialized IN (0, 1)),
  first_reader_hint_completed INTEGER NOT NULL DEFAULT 0 CHECK (first_reader_hint_completed IN (0, 1)),
  font_scale REAL NOT NULL DEFAULT 1.0 CHECK (font_scale >= 0.75 AND font_scale <= 2.0),
  line_height REAL NOT NULL DEFAULT 1.6 CHECK (line_height >= 1.2 AND line_height <= 2.4),
  reader_width REAL NOT NULL DEFAULT 72 CHECK (reader_width >= 40 AND reader_width <= 120),
  pdf_zoom REAL NOT NULL DEFAULT 1.0 CHECK (pdf_zoom >= 0.5 AND pdf_zoom <= 3.0)
);

INSERT INTO app_settings_replanned (
  id,
  onboarding_completed,
  active_provider_profile_id,
  theme,
  context_mode,
  ui_language,
  ui_language_initialized,
  first_reader_hint_completed,
  font_scale,
  line_height,
  reader_width,
  pdf_zoom
)
SELECT
  id,
  onboarding_completed,
  active_provider_profile_id,
  theme,
  context_mode,
  CASE
    WHEN ui_language IN ('zh-CN', 'zh-TW', 'en') THEN ui_language
    ELSE 'en'
  END,
  CASE
    WHEN ui_language IN ('zh-CN', 'zh-TW', 'en') THEN 1
    ELSE 0
  END,
  0,
  font_scale,
  line_height,
  reader_width,
  pdf_zoom
FROM app_settings;

DROP TABLE app_settings;
ALTER TABLE app_settings_replanned RENAME TO app_settings;
