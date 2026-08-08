ALTER TABLE provider_profiles
ADD COLUMN kimi_api_region TEXT
CHECK (kimi_api_region IS NULL OR kimi_api_region IN ('cn', 'international'));

UPDATE provider_profiles
SET kimi_api_region = 'cn'
WHERE provider_kind = 'kimi' AND kimi_api_region IS NULL;

ALTER TABLE book_extractions
ADD COLUMN kimi_api_region TEXT
CHECK (kimi_api_region IS NULL OR kimi_api_region IN ('cn', 'international'));

UPDATE book_extractions
SET kimi_api_region = COALESCE(
  (
    SELECT profile.kimi_api_region
    FROM provider_profiles AS profile
    WHERE profile.id = book_extractions.provider_profile_id
  ),
  'cn'
)
WHERE provider_kind = 'kimi' AND kimi_api_region IS NULL;

ALTER TABLE provider_remote_resources
ADD COLUMN kimi_api_region TEXT
CHECK (kimi_api_region IS NULL OR kimi_api_region IN ('cn', 'international'));

UPDATE provider_remote_resources
SET kimi_api_region = COALESCE(
  (
    SELECT extraction.kimi_api_region
    FROM book_extractions AS extraction
    WHERE extraction.id = provider_remote_resources.extraction_id
  ),
  (
    SELECT profile.kimi_api_region
    FROM provider_profiles AS profile
    WHERE profile.id = provider_remote_resources.provider_profile_id
  ),
  'cn'
)
WHERE provider_kind = 'kimi' AND kimi_api_region IS NULL;
