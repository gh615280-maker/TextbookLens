CREATE TABLE provider_credential_key_migration_guard (
  invalid_count INTEGER NOT NULL CHECK (invalid_count = 0)
);

INSERT INTO provider_credential_key_migration_guard (invalid_count)
SELECT COUNT(*)
FROM provider_profiles
WHERE length(id) != 36
   OR substr(id, 9, 1) != '-'
   OR substr(id, 14, 1) != '-'
   OR substr(id, 19, 1) != '-'
   OR substr(id, 24, 1) != '-'
   OR length(replace(id, '-', '')) != 32
   OR lower(replace(id, '-', '')) GLOB '*[^0-9a-f]*'
   OR credential_key != 'textbooklens/' || lower(id);

DROP TABLE provider_credential_key_migration_guard;

CREATE TABLE provider_profiles_v7 (
  id TEXT PRIMARY KEY NOT NULL,
  provider_kind TEXT NOT NULL CHECK (provider_kind IN ('openai', 'gemini', 'anthropic', 'deepseek', 'kimi')),
  display_name TEXT NOT NULL,
  model_id TEXT NOT NULL,
  context_window_tokens INTEGER NOT NULL CHECK (context_window_tokens > 0),
  is_active INTEGER NOT NULL DEFAULT 0 CHECK (is_active IN (0, 1)),
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  validated_at TEXT
);

INSERT INTO provider_profiles_v7 (
  id,
  provider_kind,
  display_name,
  model_id,
  context_window_tokens,
  is_active,
  created_at,
  updated_at,
  validated_at
)
SELECT
  id,
  provider_kind,
  display_name,
  model_id,
  context_window_tokens,
  is_active,
  created_at,
  updated_at,
  validated_at
FROM provider_profiles;

CREATE TABLE app_settings_v7 (
  id INTEGER PRIMARY KEY NOT NULL CHECK (id = 1),
  onboarding_completed INTEGER NOT NULL DEFAULT 0 CHECK (onboarding_completed IN (0, 1)),
  active_provider_profile_id TEXT REFERENCES provider_profiles_v7(id) ON DELETE SET NULL,
  default_learning_profile_id TEXT REFERENCES provider_profiles_v7(id) ON DELETE SET NULL,
  default_vision_profile_id TEXT REFERENCES provider_profiles_v7(id) ON DELETE SET NULL,
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

INSERT INTO app_settings_v7 (
  id,
  onboarding_completed,
  active_provider_profile_id,
  default_learning_profile_id,
  default_vision_profile_id,
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
  active_provider_profile_id,
  NULL,
  theme,
  context_mode,
  ui_language,
  ui_language_initialized,
  first_reader_hint_completed,
  font_scale,
  line_height,
  reader_width,
  pdf_zoom
FROM app_settings;

DROP TABLE app_settings;
DROP TABLE provider_profiles;
ALTER TABLE provider_profiles_v7 RENAME TO provider_profiles;
ALTER TABLE app_settings_v7 RENAME TO app_settings;

CREATE UNIQUE INDEX provider_profiles_at_most_one_active
ON provider_profiles (is_active)
WHERE is_active = 1;

CREATE TABLE provider_operation_consents (
  profile_id TEXT NOT NULL REFERENCES provider_profiles(id) ON DELETE CASCADE,
  category TEXT NOT NULL CHECK (category IN ('image_send', 'ai_index', 'cost_risk')),
  decision TEXT NOT NULL CHECK (decision IN ('ask', 'skip_prompt')),
  updated_at TEXT NOT NULL,
  PRIMARY KEY (profile_id, category)
);

INSERT INTO provider_operation_consents (profile_id, category, decision, updated_at)
SELECT provider_profiles.id, categories.category, 'ask', provider_profiles.updated_at
FROM provider_profiles
CROSS JOIN (
  SELECT 'image_send' AS category
  UNION ALL SELECT 'ai_index'
  UNION ALL SELECT 'cost_risk'
) AS categories;
