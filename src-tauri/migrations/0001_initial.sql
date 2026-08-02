PRAGMA foreign_keys = ON;

CREATE TABLE books (
  id TEXT PRIMARY KEY NOT NULL,
  sha256 TEXT NOT NULL UNIQUE,
  title TEXT NOT NULL,
  author TEXT,
  language TEXT,
  format TEXT NOT NULL CHECK (format IN ('pdf', 'epub', 'docx')),
  original_filename TEXT NOT NULL,
  stored_path TEXT NOT NULL,
  import_status TEXT NOT NULL CHECK (import_status IN ('copying', 'parsing', 'indexing', 'ready', 'failed')),
  import_error_code TEXT,
  import_error_message TEXT,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  last_opened_at TEXT,
  reading_progress REAL NOT NULL DEFAULT 0 CHECK (reading_progress >= 0 AND reading_progress <= 1),
  last_locator_json TEXT
);

CREATE TABLE sections (
  id TEXT PRIMARY KEY NOT NULL,
  book_id TEXT NOT NULL REFERENCES books(id) ON DELETE CASCADE,
  parent_id TEXT REFERENCES sections(id) ON DELETE CASCADE,
  ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
  title TEXT NOT NULL,
  locator_json TEXT NOT NULL,
  UNIQUE (book_id, ordinal)
);

CREATE TABLE blocks (
  id TEXT PRIMARY KEY NOT NULL,
  book_id TEXT NOT NULL REFERENCES books(id) ON DELETE CASCADE,
  section_id TEXT NOT NULL REFERENCES sections(id) ON DELETE CASCADE,
  ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
  kind TEXT NOT NULL,
  plain_text TEXT NOT NULL,
  locator_json TEXT NOT NULL,
  UNIQUE (section_id, ordinal)
);

CREATE TABLE search_chunks (
  id TEXT PRIMARY KEY NOT NULL,
  book_id TEXT NOT NULL REFERENCES books(id) ON DELETE CASCADE,
  section_id TEXT NOT NULL REFERENCES sections(id) ON DELETE CASCADE,
  ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
  text TEXT NOT NULL,
  locator_json TEXT NOT NULL,
  token_estimate INTEGER NOT NULL CHECK (token_estimate >= 0),
  UNIQUE (section_id, ordinal)
);

CREATE VIRTUAL TABLE search_chunks_fts USING fts5(
  text,
  content='search_chunks',
  content_rowid='rowid',
  tokenize='trigram'
);
CREATE TRIGGER search_chunks_ai AFTER INSERT ON search_chunks BEGIN
  INSERT INTO search_chunks_fts(rowid, text) VALUES (new.rowid, new.text);
END;
CREATE TRIGGER search_chunks_ad AFTER DELETE ON search_chunks BEGIN
  INSERT INTO search_chunks_fts(search_chunks_fts, rowid, text)
  VALUES ('delete', old.rowid, old.text);
END;
CREATE TRIGGER search_chunks_au AFTER UPDATE OF text ON search_chunks BEGIN
  INSERT INTO search_chunks_fts(search_chunks_fts, rowid, text)
  VALUES ('delete', old.rowid, old.text);
  INSERT INTO search_chunks_fts(rowid, text) VALUES (new.rowid, new.text);
END;

CREATE TABLE conversations (
  id TEXT PRIMARY KEY NOT NULL,
  book_id TEXT NOT NULL REFERENCES books(id) ON DELETE CASCADE,
  section_id TEXT REFERENCES sections(id) ON DELETE CASCADE,
  scope TEXT NOT NULL CHECK (scope IN ('selection', 'book')),
  anchor_json TEXT,
  selected_text TEXT,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  CHECK (
    (scope = 'selection' AND section_id IS NOT NULL AND anchor_json IS NOT NULL AND selected_text IS NOT NULL AND length(trim(selected_text)) > 0)
    OR (scope = 'book' AND section_id IS NULL AND anchor_json IS NULL AND selected_text IS NULL)
  )
);

CREATE TABLE annotations (
  id TEXT PRIMARY KEY NOT NULL,
  book_id TEXT NOT NULL REFERENCES books(id) ON DELETE CASCADE,
  section_id TEXT REFERENCES sections(id) ON DELETE CASCADE,
  kind TEXT NOT NULL CHECK (kind IN ('ai_conversation', 'note')),
  anchor_json TEXT,
  selected_text TEXT,
  note_text TEXT,
  conversation_id TEXT REFERENCES conversations(id) ON DELETE CASCADE,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  CHECK (
    (kind = 'note' AND note_text IS NOT NULL AND length(trim(note_text)) > 0 AND conversation_id IS NULL)
    OR (kind = 'ai_conversation' AND conversation_id IS NOT NULL AND note_text IS NULL)
  )
);

CREATE TRIGGER annotations_ai_ad AFTER DELETE ON annotations
WHEN old.kind = 'ai_conversation' AND old.conversation_id IS NOT NULL BEGIN
  DELETE FROM conversations WHERE id = old.conversation_id;
END;

CREATE TABLE messages (
  id TEXT PRIMARY KEY NOT NULL,
  conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
  ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
  role TEXT NOT NULL CHECK (role IN ('user', 'assistant')),
  action TEXT NOT NULL CHECK (action IN ('explain', 'example', 'derive', 'translate', 'ask', 'continue', 'overview')),
  content TEXT NOT NULL,
  provider_id TEXT,
  model_id TEXT,
  citations_json TEXT,
  created_at TEXT NOT NULL,
  CHECK (
    (role = 'assistant' AND provider_id IS NOT NULL AND model_id IS NOT NULL)
    OR (role = 'user' AND provider_id IS NULL AND model_id IS NULL)
  ),
  UNIQUE (conversation_id, ordinal)
);

CREATE TABLE provider_profiles (
  id TEXT PRIMARY KEY NOT NULL,
  provider_kind TEXT NOT NULL CHECK (provider_kind IN ('openai', 'gemini', 'anthropic', 'deepseek', 'kimi')),
  display_name TEXT NOT NULL,
  model_id TEXT NOT NULL,
  context_window_tokens INTEGER NOT NULL CHECK (context_window_tokens > 0),
  credential_key TEXT NOT NULL UNIQUE,
  is_active INTEGER NOT NULL DEFAULT 0 CHECK (is_active IN (0, 1)),
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

CREATE TABLE app_settings (
  id INTEGER PRIMARY KEY NOT NULL CHECK (id = 1),
  onboarding_completed INTEGER NOT NULL DEFAULT 0 CHECK (onboarding_completed IN (0, 1)),
  active_provider_profile_id TEXT REFERENCES provider_profiles(id) ON DELETE SET NULL,
  theme TEXT NOT NULL DEFAULT 'system' CHECK (theme IN ('light', 'dark', 'system')),
  context_mode TEXT NOT NULL DEFAULT 'standard' CHECK (context_mode IN ('standard', 'long')),
  ui_language TEXT NOT NULL DEFAULT 'zh-CN'
);

INSERT INTO app_settings (id, onboarding_completed, theme, context_mode, ui_language)
VALUES (1, 0, 'system', 'standard', 'zh-CN');
