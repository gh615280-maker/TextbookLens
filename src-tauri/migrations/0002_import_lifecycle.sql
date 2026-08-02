-- no-transaction
PRAGMA foreign_keys = OFF;

CREATE TABLE books_phase2 (
  id TEXT PRIMARY KEY NOT NULL,
  sha256 TEXT UNIQUE,
  title TEXT NOT NULL,
  author TEXT,
  language TEXT,
  format TEXT NOT NULL CHECK (format IN ('pdf', 'epub', 'docx')),
  original_filename TEXT NOT NULL,
  stored_path TEXT,
  import_status TEXT NOT NULL CHECK (import_status IN ('queued', 'copying', 'parsing', 'indexing', 'ready', 'failed')),
  import_error_code TEXT,
  import_error_message TEXT,
  import_error_stage TEXT CHECK (import_error_stage IN ('copying', 'parsing', 'indexing')),
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  last_opened_at TEXT,
  reading_progress REAL NOT NULL DEFAULT 0 CHECK (reading_progress >= 0 AND reading_progress <= 1),
  last_locator_json TEXT,
  CHECK (
    (sha256 IS NULL AND stored_path IS NULL)
    OR (sha256 IS NOT NULL AND stored_path IS NOT NULL)
  )
);

INSERT INTO books_phase2 (
  id,
  sha256,
  title,
  author,
  language,
  format,
  original_filename,
  stored_path,
  import_status,
  import_error_code,
  import_error_message,
  import_error_stage,
  created_at,
  updated_at,
  last_opened_at,
  reading_progress,
  last_locator_json
)
SELECT
  id,
  CASE WHEN stored_path = '' OR sha256 LIKE 'pending:%' THEN NULL ELSE sha256 END,
  title,
  author,
  language,
  format,
  original_filename,
  CASE WHEN stored_path = '' OR sha256 LIKE 'pending:%' THEN NULL ELSE stored_path END,
  import_status,
  import_error_code,
  import_error_message,
  CASE
    WHEN import_status = 'failed' AND import_error_message LIKE 'copying:%' THEN 'copying'
    WHEN import_status = 'failed' AND import_error_message LIKE 'parsing:%' THEN 'parsing'
    WHEN import_status = 'failed' AND import_error_message LIKE 'indexing:%' THEN 'indexing'
    ELSE NULL
  END,
  created_at,
  updated_at,
  last_opened_at,
  reading_progress,
  last_locator_json
FROM books;

DROP TABLE books;
ALTER TABLE books_phase2 RENAME TO books;

PRAGMA foreign_keys = ON;
