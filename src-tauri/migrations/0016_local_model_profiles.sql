-- no-transaction
PRAGMA foreign_keys = OFF;
BEGIN IMMEDIATE;

CREATE TABLE provider_profiles_v16 (
  id TEXT PRIMARY KEY NOT NULL,
  provider_kind TEXT NOT NULL CHECK (provider_kind IN ('openai', 'gemini', 'anthropic', 'deepseek', 'kimi', 'ollama', 'lm_studio')),
  display_name TEXT NOT NULL,
  model_id TEXT NOT NULL,
  context_window_tokens INTEGER NOT NULL CHECK (context_window_tokens > 0),
  is_active INTEGER NOT NULL DEFAULT 0 CHECK (is_active IN (0, 1)),
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  validated_at TEXT,
  kimi_api_region TEXT CHECK (kimi_api_region IS NULL OR kimi_api_region IN ('cn', 'international')),
  local_port INTEGER CHECK (local_port BETWEEN 1 AND 65535),
  CHECK ((provider_kind IN ('ollama', 'lm_studio')) = (local_port IS NOT NULL))
);

INSERT INTO provider_profiles_v16 (
  id, provider_kind, display_name, model_id, context_window_tokens,
  is_active, created_at, updated_at, validated_at, kimi_api_region
)
SELECT id, provider_kind, display_name, model_id, context_window_tokens,
       is_active, created_at, updated_at, validated_at, kimi_api_region
FROM provider_profiles;

DROP TABLE provider_profiles;
ALTER TABLE provider_profiles_v16 RENAME TO provider_profiles;
CREATE UNIQUE INDEX provider_profiles_at_most_one_active
ON provider_profiles (is_active) WHERE is_active = 1;
CREATE UNIQUE INDEX provider_profiles_local_identity
ON provider_profiles (provider_kind, local_port, model_id) WHERE local_port IS NOT NULL;

CREATE TABLE local_profiles_foreign_key_guard (
  invalid_count INTEGER NOT NULL CHECK (invalid_count = 0)
);
INSERT INTO local_profiles_foreign_key_guard SELECT COUNT(*) FROM pragma_foreign_key_check;
DROP TABLE local_profiles_foreign_key_guard;

COMMIT;
PRAGMA foreign_keys = ON;
