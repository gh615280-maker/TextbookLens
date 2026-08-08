ALTER TABLE annotations
ADD COLUMN summary_text TEXT
CHECK (
  summary_text IS NULL
  OR (
    length(trim(summary_text)) BETWEEN 1 AND 512
    AND instr(summary_text, char(0)) = 0
    AND instr(summary_text, char(13)) = 0
  )
);
