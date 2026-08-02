CREATE TABLE blocks_canonical (
  id TEXT PRIMARY KEY NOT NULL,
  book_id TEXT NOT NULL REFERENCES books(id) ON DELETE CASCADE,
  section_id TEXT NOT NULL REFERENCES sections(id) ON DELETE CASCADE,
  ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
  kind TEXT NOT NULL CHECK (kind IN ('heading', 'paragraph', 'list', 'table', 'caption', 'equation')),
  plain_text TEXT NOT NULL,
  locator_json TEXT NOT NULL,
  UNIQUE (section_id, ordinal)
);

INSERT INTO blocks_canonical (
  id,
  book_id,
  section_id,
  ordinal,
  kind,
  plain_text,
  locator_json
)
SELECT
  id,
  book_id,
  section_id,
  ordinal,
  CASE kind
    WHEN 'list_item' THEN 'list'
    WHEN 'figure_caption' THEN 'caption'
    WHEN 'code' THEN 'equation'
    ELSE kind
  END,
  plain_text,
  locator_json
FROM blocks;

DROP TABLE blocks;
ALTER TABLE blocks_canonical RENAME TO blocks;
