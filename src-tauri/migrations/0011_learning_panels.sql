-- no-transaction
PRAGMA foreign_keys = OFF;

BEGIN IMMEDIATE;

CREATE TABLE conversations_v11 (
  id TEXT PRIMARY KEY NOT NULL,
  book_id TEXT NOT NULL REFERENCES books(id) ON DELETE CASCADE,
  section_id TEXT REFERENCES sections(id) ON DELETE CASCADE,
  scope TEXT NOT NULL CHECK (scope IN ('selection', 'book')),
  anchor_kind TEXT CHECK (anchor_kind IN ('text', 'region')),
  anchor_json TEXT,
  selected_text TEXT,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  CHECK (
    (
      scope = 'selection'
      AND section_id IS NOT NULL
      AND anchor_json IS NOT NULL
      AND (
        (
          anchor_kind = 'text'
          AND selected_text IS NOT NULL
          AND length(trim(selected_text)) > 0
        )
        OR (
          anchor_kind = 'region'
          AND (
            selected_text IS NULL
            OR length(trim(selected_text)) > 0
          )
        )
      )
    )
    OR (
      scope = 'book'
      AND section_id IS NULL
      AND anchor_kind IS NULL
      AND anchor_json IS NULL
      AND selected_text IS NULL
    )
  )
);

INSERT INTO conversations_v11 (
  id,
  book_id,
  section_id,
  scope,
  anchor_kind,
  anchor_json,
  selected_text,
  created_at,
  updated_at
)
SELECT
  id,
  book_id,
  section_id,
  scope,
  CASE WHEN scope = 'selection' THEN 'text' ELSE NULL END,
  anchor_json,
  selected_text,
  created_at,
  updated_at
FROM conversations;

DROP TRIGGER annotations_ai_ad;
DROP TABLE conversations;
ALTER TABLE conversations_v11 RENAME TO conversations;

CREATE TRIGGER annotations_ai_ad AFTER DELETE ON annotations
WHEN old.kind = 'ai_conversation' AND old.conversation_id IS NOT NULL BEGIN
  DELETE FROM conversations WHERE id = old.conversation_id;
END;

CREATE TABLE panel_preferences (
  id INTEGER PRIMARY KEY NOT NULL CHECK (id = 1),
  last_x_ratio REAL NOT NULL DEFAULT 0.5 CHECK (
    typeof(last_x_ratio) IN ('integer', 'real')
    AND last_x_ratio >= 0.0
    AND last_x_ratio <= 1.0
  ),
  last_y_ratio REAL NOT NULL DEFAULT 0.5 CHECK (
    typeof(last_y_ratio) IN ('integer', 'real')
    AND last_y_ratio >= 0.0
    AND last_y_ratio <= 1.0
  ),
  width_px REAL NOT NULL DEFAULT 400.0 CHECK (
    typeof(width_px) IN ('integer', 'real')
    AND width_px >= 320.0
    AND width_px <= 8192.0
  ),
  height_px REAL NOT NULL DEFAULT 520.0 CHECK (
    typeof(height_px) IN ('integer', 'real')
    AND height_px >= 240.0
    AND height_px <= 8192.0
  ),
  updated_at TEXT NOT NULL CHECK (
    updated_at GLOB '????-??-??T??:??:??.???Z'
    AND strftime('%Y-%m-%dT%H:%M:%fZ', updated_at) IS NOT NULL
    AND strftime('%Y-%m-%dT%H:%M:%fZ', updated_at) = updated_at
  )
);

INSERT INTO panel_preferences (id, updated_at)
VALUES (1, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'));

CREATE TRIGGER panel_preferences_keep_singleton
BEFORE DELETE ON panel_preferences
BEGIN
  SELECT RAISE(ABORT, 'panel_preferences singleton cannot be deleted');
END;

CREATE TABLE learning_panels_foreign_key_guard (
  invalid_count INTEGER NOT NULL CHECK (invalid_count = 0)
);

INSERT INTO learning_panels_foreign_key_guard (invalid_count)
SELECT COUNT(*) FROM pragma_foreign_key_check;

DROP TABLE learning_panels_foreign_key_guard;

COMMIT;

PRAGMA foreign_keys = ON;
