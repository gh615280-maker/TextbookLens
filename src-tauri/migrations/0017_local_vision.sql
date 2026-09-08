ALTER TABLE provider_profiles ADD COLUMN local_vision INTEGER NOT NULL DEFAULT 0
CHECK (local_vision IN (0, 1) AND (local_vision = 0 OR provider_kind = 'ollama'));
