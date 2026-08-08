UPDATE books
SET title = CASE
  WHEN format = 'pdf'
    AND lower(original_filename) LIKE '%.pdf'
    AND length(original_filename) > 4
    THEN substr(original_filename, 1, length(original_filename) - 4)
  WHEN format = 'epub'
    AND lower(original_filename) LIKE '%.epub'
    AND length(original_filename) > 5
    THEN substr(original_filename, 1, length(original_filename) - 5)
  WHEN format = 'docx'
    AND lower(original_filename) LIKE '%.docx'
    AND length(original_filename) > 5
    THEN substr(original_filename, 1, length(original_filename) - 5)
  ELSE original_filename
END
WHERE lower(trim(title)) IN (
  '未命名',
  '未命名 pdf',
  '未命名 epub',
  '未命名 docx',
  'untitled',
  'untitled pdf',
  'untitled epub',
  'untitled docx'
);
