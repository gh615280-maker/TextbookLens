CREATE TABLE book_extractions (
  id TEXT PRIMARY KEY NOT NULL,
  book_id TEXT NOT NULL REFERENCES books(id) ON DELETE CASCADE,
  source_sha256 TEXT NOT NULL,
  provider_profile_id TEXT REFERENCES provider_profiles(id) ON DELETE SET NULL,
  provider_kind TEXT NOT NULL CHECK (provider_kind = 'kimi'),
  model_id TEXT NOT NULL,
  extraction_version TEXT NOT NULL,
  credential_fingerprint TEXT NOT NULL,
  status TEXT NOT NULL CHECK (status IN (
    'queued', 'uploading', 'processing', 'downloading', 'indexing',
    'ready', 'cancelling', 'cancelled', 'failed'
  )),
  attempt_count INTEGER NOT NULL DEFAULT 0 CHECK (attempt_count >= 0),
  normalized_text TEXT,
  content_sha256 TEXT,
  content_chars INTEGER NOT NULL DEFAULT 0 CHECK (content_chars >= 0),
  safe_error_code TEXT,
  retryable INTEGER NOT NULL DEFAULT 0 CHECK (retryable IN (0, 1)),
  cancel_requested_at TEXT,
  completed_at TEXT,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  UNIQUE(source_sha256, provider_kind, extraction_version),
  FOREIGN KEY (book_id, source_sha256) REFERENCES books(id, sha256) ON DELETE CASCADE
);

CREATE INDEX book_extractions_book_status
ON book_extractions(book_id, status, updated_at);

CREATE TABLE extraction_chunks (
  id TEXT PRIMARY KEY NOT NULL,
  extraction_id TEXT NOT NULL REFERENCES book_extractions(id) ON DELETE CASCADE,
  book_id TEXT NOT NULL REFERENCES books(id) ON DELETE CASCADE,
  ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
  char_start INTEGER NOT NULL CHECK (char_start >= 0),
  char_end INTEGER NOT NULL CHECK (char_end > char_start),
  paragraph_start INTEGER NOT NULL CHECK (paragraph_start >= 0),
  paragraph_end INTEGER NOT NULL CHECK (paragraph_end >= paragraph_start),
  text TEXT NOT NULL CHECK (length(trim(text)) > 0),
  text_fingerprint TEXT NOT NULL,
  token_estimate INTEGER NOT NULL CHECK (token_estimate > 0),
  locator_json TEXT NOT NULL CHECK (json_valid(locator_json)),
  created_at TEXT NOT NULL,
  UNIQUE(extraction_id, ordinal)
);

CREATE INDEX extraction_chunks_book_ordinal
ON extraction_chunks(book_id, ordinal);

CREATE VIRTUAL TABLE extraction_chunks_fts USING fts5(
  text,
  content='extraction_chunks',
  content_rowid='rowid',
  tokenize='trigram'
);

CREATE TRIGGER extraction_chunks_ai AFTER INSERT ON extraction_chunks BEGIN
  INSERT INTO extraction_chunks_fts(rowid, text) VALUES (new.rowid, new.text);
END;
CREATE TRIGGER extraction_chunks_ad AFTER DELETE ON extraction_chunks BEGIN
  INSERT INTO extraction_chunks_fts(extraction_chunks_fts, rowid, text)
  VALUES ('delete', old.rowid, old.text);
END;
CREATE TRIGGER extraction_chunks_au AFTER UPDATE OF text ON extraction_chunks BEGIN
  INSERT INTO extraction_chunks_fts(extraction_chunks_fts, rowid, text)
  VALUES ('delete', old.rowid, old.text);
  INSERT INTO extraction_chunks_fts(rowid, text) VALUES (new.rowid, new.text);
END;

ALTER TABLE provider_remote_resources
ADD COLUMN extraction_id TEXT REFERENCES book_extractions(id) ON DELETE SET NULL;

CREATE INDEX provider_remote_resources_extraction
ON provider_remote_resources(extraction_id, cleanup_status);
