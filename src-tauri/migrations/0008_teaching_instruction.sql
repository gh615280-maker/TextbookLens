CREATE TABLE teaching_preferences (
  id INTEGER PRIMARY KEY NOT NULL CHECK (id = 1),
  instruction TEXT NOT NULL DEFAULT '' CHECK (
    length(instruction) <= 1000
    AND instr(instruction, char(13)) = 0
  ),
  revision INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0),
  updated_at TEXT NOT NULL CHECK (
    updated_at GLOB '????-??-??T??:??:??.???Z'
    AND strftime('%Y-%m-%dT%H:%M:%fZ', updated_at) = updated_at
  )
);

INSERT INTO teaching_preferences (id, instruction, revision, updated_at)
VALUES (1, '', 0, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'));

CREATE TRIGGER teaching_preferences_keep_singleton
BEFORE DELETE ON teaching_preferences
BEGIN
  SELECT RAISE(ABORT, 'teaching_preferences singleton cannot be deleted');
END;
