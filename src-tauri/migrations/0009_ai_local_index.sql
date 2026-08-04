CREATE UNIQUE INDEX books_id_sha256
ON books (id, sha256);

CREATE TABLE index_runs (
  id TEXT PRIMARY KEY NOT NULL CHECK (
    length(id) = 36
    AND substr(id, 9, 1) = '-'
    AND substr(id, 14, 1) = '-'
    AND substr(id, 19, 1) = '-'
    AND substr(id, 24, 1) = '-'
    AND length(replace(id, '-', '')) = 32
    AND replace(id, '-', '') NOT GLOB '*[^0-9a-f]*'
  ),
  book_id TEXT NOT NULL,
  source_sha256 TEXT NOT NULL CHECK (
    length(source_sha256) = 64
    AND source_sha256 NOT GLOB '*[^0-9a-f]*'
  ),
  provider_profile_id TEXT REFERENCES provider_profiles(id) ON DELETE SET NULL,
  provider_kind TEXT NOT NULL CHECK (
    provider_kind IN ('openai', 'gemini', 'anthropic', 'deepseek', 'kimi')
  ),
  model_id TEXT NOT NULL CHECK (
    length(trim(model_id)) BETWEEN 1 AND 256
    AND instr(model_id, char(13)) = 0
    AND instr(model_id, char(10)) = 0
  ),
  analysis_schema_version TEXT NOT NULL CHECK (
    length(trim(analysis_schema_version)) BETWEEN 1 AND 64
    AND instr(analysis_schema_version, char(13)) = 0
    AND instr(analysis_schema_version, char(10)) = 0
  ),
  render_version TEXT NOT NULL CHECK (
    length(trim(render_version)) BETWEEN 1 AND 64
    AND instr(render_version, char(13)) = 0
    AND instr(render_version, char(10)) = 0
  ),
  parser_version TEXT NOT NULL CHECK (
    length(trim(parser_version)) BETWEEN 1 AND 64
    AND instr(parser_version, char(13)) = 0
    AND instr(parser_version, char(10)) = 0
  ),
  status TEXT NOT NULL CHECK (
    status IN ('running', 'paused', 'cancelling', 'cancelled', 'completed')
  ),
  created_at TEXT NOT NULL CHECK (
    strftime('%Y-%m-%dT%H:%M:%fZ', created_at) IS NOT NULL
    AND strftime('%Y-%m-%dT%H:%M:%fZ', created_at) = created_at
  ),
  updated_at TEXT NOT NULL CHECK (
    strftime('%Y-%m-%dT%H:%M:%fZ', updated_at) IS NOT NULL
    AND strftime('%Y-%m-%dT%H:%M:%fZ', updated_at) = updated_at
  ),
  paused_at TEXT CHECK (
    paused_at IS NULL
    OR (
      strftime('%Y-%m-%dT%H:%M:%fZ', paused_at) IS NOT NULL
      AND strftime('%Y-%m-%dT%H:%M:%fZ', paused_at) = paused_at
    )
  ),
  cancel_requested_at TEXT CHECK (
    cancel_requested_at IS NULL
    OR (
      strftime('%Y-%m-%dT%H:%M:%fZ', cancel_requested_at) IS NOT NULL
      AND strftime('%Y-%m-%dT%H:%M:%fZ', cancel_requested_at) = cancel_requested_at
    )
  ),
  completed_at TEXT CHECK (
    completed_at IS NULL
    OR (
      strftime('%Y-%m-%dT%H:%M:%fZ', completed_at) IS NOT NULL
      AND strftime('%Y-%m-%dT%H:%M:%fZ', completed_at) = completed_at
    )
  ),
  FOREIGN KEY (book_id, source_sha256)
    REFERENCES books(id, sha256) ON DELETE CASCADE,
  UNIQUE (id, book_id),
  CHECK ((status = 'paused') = (paused_at IS NOT NULL)),
  CHECK (
    (status IN ('cancelling', 'cancelled')) = (cancel_requested_at IS NOT NULL)
  ),
  CHECK (
    (status IN ('cancelled', 'completed')) = (completed_at IS NOT NULL)
  )
);

CREATE INDEX index_runs_book_status
ON index_runs (book_id, status, created_at);

CREATE TABLE index_pages (
  id TEXT PRIMARY KEY NOT NULL CHECK (
    length(id) = 36
    AND substr(id, 9, 1) = '-'
    AND substr(id, 14, 1) = '-'
    AND substr(id, 19, 1) = '-'
    AND substr(id, 24, 1) = '-'
    AND length(replace(id, '-', '')) = 32
    AND replace(id, '-', '') NOT GLOB '*[^0-9a-f]*'
  ),
  run_id TEXT NOT NULL,
  book_id TEXT NOT NULL,
  page_number INTEGER NOT NULL CHECK (page_number BETWEEN 1 AND 1000000),
  quality_reason TEXT NOT NULL CHECK (
    quality_reason IN (
      'reliable_text',
      'no_text',
      'very_low_text_coverage',
      'high_replacement_or_control_ratio',
      'extreme_duplicate_glyphs',
      'layout_contradiction'
    )
  ),
  status TEXT NOT NULL CHECK (
    status IN (
      'not_required',
      'queued',
      'rendering',
      'sending',
      'parsing',
      'validating',
      'indexed',
      'needs_review',
      'failed',
      'cancelled'
    )
  ),
  attempt_id TEXT CHECK (
    attempt_id IS NULL
    OR (
      length(attempt_id) = 36
      AND substr(attempt_id, 9, 1) = '-'
      AND substr(attempt_id, 14, 1) = '-'
      AND substr(attempt_id, 19, 1) = '-'
      AND substr(attempt_id, 24, 1) = '-'
      AND length(replace(attempt_id, '-', '')) = 32
      AND replace(attempt_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    )
  ),
  attempt_count INTEGER NOT NULL DEFAULT 0 CHECK (attempt_count BETWEEN 0 AND 1000000),
  attempt_started_at TEXT CHECK (
    attempt_started_at IS NULL
    OR (
      strftime('%Y-%m-%dT%H:%M:%fZ', attempt_started_at) IS NOT NULL
      AND strftime('%Y-%m-%dT%H:%M:%fZ', attempt_started_at) = attempt_started_at
    )
  ),
  local_text_sha256 TEXT CHECK (
    local_text_sha256 IS NULL
    OR (
      length(local_text_sha256) = 64
      AND local_text_sha256 NOT GLOB '*[^0-9a-f]*'
    )
  ),
  render_sha256 TEXT CHECK (
    render_sha256 IS NULL
    OR (
      length(render_sha256) = 64
      AND render_sha256 NOT GLOB '*[^0-9a-f]*'
    )
  ),
  response_sha256 TEXT CHECK (
    response_sha256 IS NULL
    OR (
      length(response_sha256) = 64
      AND response_sha256 NOT GLOB '*[^0-9a-f]*'
    )
  ),
  content_sha256 TEXT CHECK (
    content_sha256 IS NULL
    OR (
      length(content_sha256) = 64
      AND content_sha256 NOT GLOB '*[^0-9a-f]*'
    )
  ),
  content_version INTEGER NOT NULL DEFAULT 0 CHECK (content_version BETWEEN 0 AND 1000000),
  review_reason_code TEXT CHECK (
    review_reason_code IS NULL
    OR review_reason_code IN (
      'incomplete_content',
      'invalid_bounds',
      'severe_overlap',
      'text_contradiction',
      'malformed_table',
      'malformed_latex'
    )
  ),
  safe_error_code TEXT CHECK (
    safe_error_code IS NULL
    OR safe_error_code IN (
      'INDEX_RENDER_FAILED',
      'INDEX_PROVIDER_FAILED',
      'INDEX_RESPONSE_INVALID',
      'INDEX_VALIDATION_FAILED',
      'INDEX_ATTEMPT_INTERRUPTED',
      'INDEX_SCRATCH_MISSING',
      'INDEX_ATTEMPT_EXPIRED'
    )
  ),
  safe_error_message TEXT CHECK (
    safe_error_message IS NULL
    OR (
      length(trim(safe_error_message)) BETWEEN 1 AND 256
      AND instr(safe_error_message, char(13)) = 0
      AND instr(safe_error_message, char(10)) = 0
    )
  ),
  retryable INTEGER NOT NULL DEFAULT 0 CHECK (retryable IN (0, 1)),
  created_at TEXT NOT NULL CHECK (
    strftime('%Y-%m-%dT%H:%M:%fZ', created_at) IS NOT NULL
    AND strftime('%Y-%m-%dT%H:%M:%fZ', created_at) = created_at
  ),
  updated_at TEXT NOT NULL CHECK (
    strftime('%Y-%m-%dT%H:%M:%fZ', updated_at) IS NOT NULL
    AND strftime('%Y-%m-%dT%H:%M:%fZ', updated_at) = updated_at
  ),
  FOREIGN KEY (run_id, book_id)
    REFERENCES index_runs(id, book_id) ON DELETE CASCADE,
  UNIQUE (run_id, page_number),
  UNIQUE (id, book_id),
  UNIQUE (id, run_id, book_id),
  CHECK (
    (quality_reason = 'reliable_text' AND status = 'not_required')
    OR (quality_reason != 'reliable_text' AND status != 'not_required')
  ),
  CHECK (
    (status = 'not_required' AND attempt_id IS NULL AND attempt_count = 0)
    OR (status != 'not_required' AND attempt_id IS NOT NULL AND attempt_count > 0)
  ),
  CHECK (
    status NOT IN ('rendering', 'sending', 'parsing', 'validating')
    OR attempt_started_at IS NOT NULL
  ),
  CHECK (status != 'queued' OR attempt_started_at IS NULL),
  CHECK (status != 'sending' OR render_sha256 IS NOT NULL),
  CHECK (
    status NOT IN ('parsing', 'validating')
    OR (render_sha256 IS NOT NULL AND response_sha256 IS NOT NULL)
  ),
  CHECK (
    (status = 'needs_review') = (review_reason_code IS NOT NULL)
  ),
  CHECK (
    (status = 'failed') = (
      safe_error_code IS NOT NULL
      AND safe_error_message IS NOT NULL
    )
  ),
  CHECK (status = 'failed' OR retryable = 0),
  CHECK (
    status NOT IN ('indexed', 'needs_review')
    OR (content_version > 0 AND content_sha256 IS NOT NULL)
  )
);

CREATE INDEX index_pages_run_status
ON index_pages (run_id, status, page_number);

CREATE INDEX index_pages_book_status
ON index_pages (book_id, status, page_number);

CREATE TABLE index_page_blocks (
  id TEXT PRIMARY KEY NOT NULL CHECK (
    length(id) = 36
    AND substr(id, 9, 1) = '-'
    AND substr(id, 14, 1) = '-'
    AND substr(id, 19, 1) = '-'
    AND substr(id, 24, 1) = '-'
    AND length(replace(id, '-', '')) = 32
    AND replace(id, '-', '') NOT GLOB '*[^0-9a-f]*'
  ),
  page_id TEXT NOT NULL,
  run_id TEXT NOT NULL,
  book_id TEXT NOT NULL,
  ordinal INTEGER NOT NULL CHECK (ordinal BETWEEN 0 AND 100000),
  kind TEXT NOT NULL CHECK (
    kind IN ('title', 'paragraph', 'list', 'table', 'caption', 'formula', 'figure', 'transcript')
  ),
  plain_text TEXT CHECK (
    plain_text IS NULL
    OR (
      length(trim(plain_text)) BETWEEN 1 AND 65536
      AND instr(plain_text, char(0)) = 0
      AND instr(plain_text, char(13)) = 0
    )
  ),
  latex TEXT CHECK (
    latex IS NULL
    OR (
      length(trim(latex)) BETWEEN 1 AND 16384
      AND instr(latex, char(0)) = 0
      AND instr(latex, char(13)) = 0
    )
  ),
  table_json TEXT CHECK (
    table_json IS NULL
    OR (
      length(table_json) BETWEEN 2 AND 131072
      AND json_valid(table_json)
    )
  ),
  visual_description TEXT CHECK (
    visual_description IS NULL
    OR (
      length(trim(visual_description)) BETWEEN 1 AND 32768
      AND instr(visual_description, char(0)) = 0
      AND instr(visual_description, char(13)) = 0
    )
  ),
  bounds_x REAL,
  bounds_y REAL,
  bounds_width REAL,
  bounds_height REAL,
  source TEXT NOT NULL CHECK (source IN ('ai_transcribed', 'ai_description')),
  provenance_version INTEGER NOT NULL CHECK (provenance_version BETWEEN 1 AND 1000000),
  content_version INTEGER NOT NULL CHECK (content_version BETWEEN 1 AND 1000000),
  value_sha256 TEXT NOT NULL CHECK (
    length(value_sha256) = 64
    AND value_sha256 NOT GLOB '*[^0-9a-f]*'
  ),
  created_at TEXT NOT NULL CHECK (
    strftime('%Y-%m-%dT%H:%M:%fZ', created_at) IS NOT NULL
    AND strftime('%Y-%m-%dT%H:%M:%fZ', created_at) = created_at
  ),
  FOREIGN KEY (page_id, run_id, book_id)
    REFERENCES index_pages(id, run_id, book_id) ON DELETE CASCADE,
  UNIQUE (page_id, content_version, ordinal),
  UNIQUE (id, page_id, book_id),
  CHECK (
    plain_text IS NOT NULL
    OR latex IS NOT NULL
    OR table_json IS NOT NULL
    OR visual_description IS NOT NULL
  ),
  CHECK (
    (bounds_x IS NULL AND bounds_y IS NULL AND bounds_width IS NULL AND bounds_height IS NULL)
    OR (
      bounds_x BETWEEN 0.0 AND 1.0
      AND bounds_y BETWEEN 0.0 AND 1.0
      AND bounds_width BETWEEN 0.0 AND 1.0
      AND bounds_height BETWEEN 0.0 AND 1.0
      AND bounds_x + bounds_width <= 1.0
      AND bounds_y + bounds_height <= 1.0
    )
  ),
  CHECK (source != 'ai_description' OR visual_description IS NOT NULL)
);

CREATE INDEX index_page_blocks_page_version
ON index_page_blocks (page_id, content_version, ordinal);

CREATE TABLE index_corrections (
  id TEXT PRIMARY KEY NOT NULL CHECK (
    length(id) = 36
    AND substr(id, 9, 1) = '-'
    AND substr(id, 14, 1) = '-'
    AND substr(id, 19, 1) = '-'
    AND substr(id, 24, 1) = '-'
    AND length(replace(id, '-', '')) = 32
    AND replace(id, '-', '') NOT GLOB '*[^0-9a-f]*'
  ),
  book_id TEXT NOT NULL,
  page_id TEXT NOT NULL,
  target_block_id TEXT NOT NULL,
  target_content_version INTEGER NOT NULL CHECK (target_content_version BETWEEN 1 AND 1000000),
  region_x REAL,
  region_y REAL,
  region_width REAL,
  region_height REAL,
  value_kind TEXT NOT NULL CHECK (value_kind IN ('text', 'latex')),
  original_value_sha256 TEXT NOT NULL CHECK (
    length(original_value_sha256) = 64
    AND original_value_sha256 NOT GLOB '*[^0-9a-f]*'
  ),
  original_value TEXT NOT NULL CHECK (
    length(trim(original_value)) BETWEEN 1 AND 65536
    AND instr(original_value, char(0)) = 0
    AND instr(original_value, char(13)) = 0
  ),
  corrected_value TEXT NOT NULL CHECK (
    length(trim(corrected_value)) BETWEEN 1 AND 65536
    AND instr(corrected_value, char(0)) = 0
    AND instr(corrected_value, char(13)) = 0
  ),
  conflict_state TEXT NOT NULL CHECK (conflict_state IN ('active', 'conflict')),
  revision INTEGER NOT NULL DEFAULT 1 CHECK (revision BETWEEN 1 AND 1000000),
  created_at TEXT NOT NULL CHECK (
    strftime('%Y-%m-%dT%H:%M:%fZ', created_at) IS NOT NULL
    AND strftime('%Y-%m-%dT%H:%M:%fZ', created_at) = created_at
  ),
  updated_at TEXT NOT NULL CHECK (
    strftime('%Y-%m-%dT%H:%M:%fZ', updated_at) IS NOT NULL
    AND strftime('%Y-%m-%dT%H:%M:%fZ', updated_at) = updated_at
  ),
  FOREIGN KEY (page_id, book_id)
    REFERENCES index_pages(id, book_id) ON DELETE CASCADE,
  FOREIGN KEY (target_block_id, page_id, book_id)
    REFERENCES index_page_blocks(id, page_id, book_id) ON DELETE CASCADE,
  UNIQUE (page_id, target_block_id, value_kind),
  UNIQUE (id, page_id, book_id),
  CHECK (
    (region_x IS NULL AND region_y IS NULL AND region_width IS NULL AND region_height IS NULL)
    OR (
      region_x BETWEEN 0.0 AND 1.0
      AND region_y BETWEEN 0.0 AND 1.0
      AND region_width BETWEEN 0.0 AND 1.0
      AND region_height BETWEEN 0.0 AND 1.0
      AND region_x + region_width <= 1.0
      AND region_y + region_height <= 1.0
    )
  )
);

CREATE INDEX index_corrections_book_page
ON index_corrections (book_id, page_id, conflict_state);

CREATE TABLE index_search_chunks (
  id TEXT PRIMARY KEY NOT NULL CHECK (
    length(id) = 36
    AND substr(id, 9, 1) = '-'
    AND substr(id, 14, 1) = '-'
    AND substr(id, 19, 1) = '-'
    AND substr(id, 24, 1) = '-'
    AND length(replace(id, '-', '')) = 32
    AND replace(id, '-', '') NOT GLOB '*[^0-9a-f]*'
  ),
  book_id TEXT NOT NULL,
  page_id TEXT NOT NULL,
  block_id TEXT NOT NULL,
  correction_id TEXT,
  ordinal INTEGER NOT NULL CHECK (ordinal BETWEEN 0 AND 100000),
  source TEXT NOT NULL CHECK (
    source IN ('ai_transcribed', 'ai_description', 'user_corrected')
  ),
  text TEXT NOT NULL CHECK (
    length(trim(text)) BETWEEN 1 AND 65536
    AND instr(text, char(0)) = 0
    AND instr(text, char(13)) = 0
  ),
  locator_json TEXT NOT NULL CHECK (
    length(locator_json) BETWEEN 2 AND 16384
    AND json_valid(locator_json)
  ),
  token_estimate INTEGER NOT NULL CHECK (token_estimate BETWEEN 1 AND 1000000),
  content_version INTEGER NOT NULL CHECK (content_version BETWEEN 1 AND 1000000),
  created_at TEXT NOT NULL CHECK (
    strftime('%Y-%m-%dT%H:%M:%fZ', created_at) IS NOT NULL
    AND strftime('%Y-%m-%dT%H:%M:%fZ', created_at) = created_at
  ),
  FOREIGN KEY (page_id, book_id)
    REFERENCES index_pages(id, book_id) ON DELETE CASCADE,
  FOREIGN KEY (block_id, page_id, book_id)
    REFERENCES index_page_blocks(id, page_id, book_id) ON DELETE CASCADE,
  FOREIGN KEY (correction_id, page_id, book_id)
    REFERENCES index_corrections(id, page_id, book_id) ON DELETE CASCADE,
  UNIQUE (page_id, content_version, source, ordinal),
  CHECK (
    (source = 'user_corrected' AND correction_id IS NOT NULL)
    OR (source != 'user_corrected' AND correction_id IS NULL)
  )
);

CREATE INDEX index_search_chunks_book_page
ON index_search_chunks (book_id, page_id, source, ordinal);

CREATE VIRTUAL TABLE index_search_chunks_fts USING fts5(
  text,
  content='index_search_chunks',
  content_rowid='rowid',
  tokenize='trigram'
);

CREATE TRIGGER index_search_chunks_ai
AFTER INSERT ON index_search_chunks
BEGIN
  INSERT INTO index_search_chunks_fts(rowid, text)
  VALUES (new.rowid, new.text);
END;

CREATE TRIGGER index_search_chunks_ad
AFTER DELETE ON index_search_chunks
BEGIN
  INSERT INTO index_search_chunks_fts(index_search_chunks_fts, rowid, text)
  VALUES ('delete', old.rowid, old.text);
END;

CREATE TRIGGER index_search_chunks_au
AFTER UPDATE OF text ON index_search_chunks
BEGIN
  INSERT INTO index_search_chunks_fts(index_search_chunks_fts, rowid, text)
  VALUES ('delete', old.rowid, old.text);
  INSERT INTO index_search_chunks_fts(rowid, text)
  VALUES (new.rowid, new.text);
END;

CREATE TABLE provider_remote_resources (
  id TEXT PRIMARY KEY NOT NULL CHECK (
    length(id) = 36
    AND substr(id, 9, 1) = '-'
    AND substr(id, 14, 1) = '-'
    AND substr(id, 19, 1) = '-'
    AND substr(id, 24, 1) = '-'
    AND length(replace(id, '-', '')) = 32
    AND replace(id, '-', '') NOT GLOB '*[^0-9a-f]*'
  ),
  book_id TEXT REFERENCES books(id) ON DELETE SET NULL,
  run_id TEXT REFERENCES index_runs(id) ON DELETE SET NULL,
  page_id TEXT REFERENCES index_pages(id) ON DELETE SET NULL,
  provider_profile_id TEXT REFERENCES provider_profiles(id) ON DELETE SET NULL,
  provider_kind TEXT NOT NULL CHECK (
    provider_kind IN ('openai', 'gemini', 'anthropic', 'deepseek', 'kimi')
  ),
  encrypted_reference TEXT NOT NULL CHECK (
    encrypted_reference LIKE 'enc:v1:%'
    AND length(encrypted_reference) BETWEEN 8 AND 4096
    AND instr(encrypted_reference, char(0)) = 0
    AND instr(encrypted_reference, char(13)) = 0
    AND instr(encrypted_reference, char(10)) = 0
  ),
  cleanup_status TEXT NOT NULL CHECK (
    cleanup_status IN ('pending', 'cleaning', 'failed', 'succeeded', 'safely_disposed')
  ),
  cleanup_attempt_id TEXT CHECK (
    cleanup_attempt_id IS NULL
    OR (
      length(cleanup_attempt_id) = 36
      AND substr(cleanup_attempt_id, 9, 1) = '-'
      AND substr(cleanup_attempt_id, 14, 1) = '-'
      AND substr(cleanup_attempt_id, 19, 1) = '-'
      AND substr(cleanup_attempt_id, 24, 1) = '-'
      AND length(replace(cleanup_attempt_id, '-', '')) = 32
      AND replace(cleanup_attempt_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    )
  ),
  cleanup_attempt_count INTEGER NOT NULL DEFAULT 0 CHECK (
    cleanup_attempt_count BETWEEN 0 AND 1000000
  ),
  safe_error_code TEXT CHECK (
    safe_error_code IS NULL
    OR safe_error_code IN (
      'REMOTE_DELETE_FAILED',
      'REMOTE_NOT_FOUND',
      'REMOTE_AUTH_REQUIRED',
      'REMOTE_RETRY_EXHAUSTED'
    )
  ),
  safe_error_message TEXT CHECK (
    safe_error_message IS NULL
    OR (
      length(trim(safe_error_message)) BETWEEN 1 AND 256
      AND instr(safe_error_message, char(13)) = 0
      AND instr(safe_error_message, char(10)) = 0
    )
  ),
  disposition_reason TEXT CHECK (
    disposition_reason IS NULL
    OR disposition_reason IN ('provider_expired', 'user_authorized_forget')
  ),
  created_at TEXT NOT NULL CHECK (
    strftime('%Y-%m-%dT%H:%M:%fZ', created_at) IS NOT NULL
    AND strftime('%Y-%m-%dT%H:%M:%fZ', created_at) = created_at
  ),
  updated_at TEXT NOT NULL CHECK (
    strftime('%Y-%m-%dT%H:%M:%fZ', updated_at) IS NOT NULL
    AND strftime('%Y-%m-%dT%H:%M:%fZ', updated_at) = updated_at
  ),
  last_cleanup_at TEXT CHECK (
    last_cleanup_at IS NULL
    OR (
      strftime('%Y-%m-%dT%H:%M:%fZ', last_cleanup_at) IS NOT NULL
      AND strftime('%Y-%m-%dT%H:%M:%fZ', last_cleanup_at) = last_cleanup_at
    )
  ),
  CHECK (
    (cleanup_status = 'pending'
      AND cleanup_attempt_id IS NULL
      AND cleanup_attempt_count = 0
      AND safe_error_code IS NULL
      AND safe_error_message IS NULL
      AND disposition_reason IS NULL
      AND last_cleanup_at IS NULL)
    OR (cleanup_status = 'cleaning'
      AND cleanup_attempt_id IS NOT NULL
      AND cleanup_attempt_count > 0
      AND safe_error_code IS NULL
      AND safe_error_message IS NULL
      AND disposition_reason IS NULL
      AND last_cleanup_at IS NOT NULL)
    OR (cleanup_status = 'failed'
      AND cleanup_attempt_id IS NOT NULL
      AND cleanup_attempt_count > 0
      AND safe_error_code IS NOT NULL
      AND safe_error_message IS NOT NULL
      AND disposition_reason IS NULL
      AND last_cleanup_at IS NOT NULL)
    OR (cleanup_status = 'succeeded'
      AND cleanup_attempt_id IS NOT NULL
      AND cleanup_attempt_count > 0
      AND safe_error_code IS NULL
      AND safe_error_message IS NULL
      AND disposition_reason IS NULL
      AND last_cleanup_at IS NOT NULL)
    OR (cleanup_status = 'safely_disposed'
      AND cleanup_attempt_id IS NOT NULL
      AND cleanup_attempt_count > 0
      AND safe_error_code IS NULL
      AND safe_error_message IS NULL
      AND disposition_reason IS NOT NULL
      AND last_cleanup_at IS NOT NULL)
  )
);

CREATE INDEX provider_remote_resources_book_cleanup
ON provider_remote_resources (book_id, cleanup_status, updated_at);

CREATE INDEX provider_remote_resources_cleanup_retry
ON provider_remote_resources (cleanup_status, updated_at);

CREATE TRIGGER provider_remote_resources_reference_immutable
BEFORE UPDATE OF encrypted_reference ON provider_remote_resources
WHEN new.encrypted_reference != old.encrypted_reference
BEGIN
  SELECT RAISE(ABORT, 'remote resource reference is immutable');
END;

CREATE TRIGGER provider_remote_resources_cleanup_transition
BEFORE UPDATE OF cleanup_status ON provider_remote_resources
BEGIN
  SELECT RAISE(ABORT, 'invalid remote cleanup transition')
  WHERE NOT (
    (old.cleanup_status = 'pending' AND new.cleanup_status = 'cleaning')
    OR (old.cleanup_status = 'cleaning' AND new.cleanup_status IN ('failed', 'succeeded'))
    OR (old.cleanup_status = 'failed' AND new.cleanup_status IN ('cleaning', 'safely_disposed'))
  );
  SELECT RAISE(ABORT, 'remote cleanup attempt ownership did not advance')
  WHERE new.cleanup_status = 'cleaning'
    AND (
      new.cleanup_attempt_count != old.cleanup_attempt_count + 1
      OR new.cleanup_attempt_id IS NULL
      OR new.cleanup_attempt_id IS old.cleanup_attempt_id
    );
  SELECT RAISE(ABORT, 'remote cleanup terminal ownership changed')
  WHERE old.cleanup_status = 'cleaning'
    AND new.cleanup_status IN ('failed', 'succeeded')
    AND (
      new.cleanup_attempt_count != old.cleanup_attempt_count
      OR new.cleanup_attempt_id IS NOT old.cleanup_attempt_id
    );
  SELECT RAISE(ABORT, 'safe disposal changed cleanup ownership')
  WHERE new.cleanup_status = 'safely_disposed'
    AND (
      new.cleanup_attempt_count != old.cleanup_attempt_count
      OR new.cleanup_attempt_id IS NOT old.cleanup_attempt_id
    );
END;

CREATE TRIGGER provider_remote_resources_delete_only_resolved
BEFORE DELETE ON provider_remote_resources
WHEN old.cleanup_status NOT IN ('succeeded', 'safely_disposed')
BEGIN
  SELECT RAISE(ABORT, 'unresolved remote resource must be retained');
END;

CREATE TRIGGER books_require_remote_cleanup_before_delete
BEFORE DELETE ON books
BEGIN
  SELECT RAISE(ABORT, 'remote resource cleanup must be attempted before book deletion')
  WHERE EXISTS (
    SELECT 1
    FROM provider_remote_resources
    WHERE book_id = old.id
      AND cleanup_status IN ('pending', 'cleaning')
  );
  DELETE FROM provider_remote_resources
  WHERE book_id = old.id
    AND cleanup_status IN ('succeeded', 'safely_disposed');
END;
