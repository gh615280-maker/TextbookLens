ALTER TABLE provider_profiles ADD COLUMN validated_at TEXT;

UPDATE provider_profiles
SET is_active = 0
WHERE is_active = 1
  AND id != COALESCE(
    (SELECT active_provider_profile_id FROM app_settings WHERE id = 1),
    (SELECT id FROM provider_profiles WHERE is_active = 1 ORDER BY created_at, id LIMIT 1)
  );

UPDATE app_settings
SET active_provider_profile_id = (
  SELECT id FROM provider_profiles WHERE is_active = 1 ORDER BY created_at, id LIMIT 1
)
WHERE id = 1;

CREATE UNIQUE INDEX provider_profiles_at_most_one_active
ON provider_profiles (is_active)
WHERE is_active = 1;
