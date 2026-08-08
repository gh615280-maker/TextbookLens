# Index quality and provenance

Ordinary PDF, EPUB, and DOCX parsing and search are local. PDF pages with unreliable text can be flagged for optional AI-assisted local indexing; this is not background OCR or a whole-book upload.

## Exceptional-page indexing

You may decline AI indexing for a scanned/low-quality PDF and continue reading. If approved, the confirmation names provider, model, affected page count, and cost risk. The app renders only exceptional pages locally, sends bounded small batches, validates the response locally, and commits each page independently. Failed, cancelled, or invalid results never become completed indexes. A `partial` or `needs_review` book remains readable; retry one failed page without repeating successful pages. Completed results and FTS remain usable offline.

## Sources and corrections

| Label             | Meaning                                               |
| ----------------- | ----------------------------------------------------- |
| `local_text`      | Reliable text from the imported source                |
| `ai_transcribed`  | AI transcription of visible text or formula           |
| `ai_description`  | Figure/table/layout description; not source quotation |
| `user_corrected`  | Your explicit local override                          |
| `user_note`       | Your local note                                       |
| `history_summary` | Local summary/metadata for saved history              |

The quality page defaults to review/failed pages and presents source page plus editable result. Saving a correction creates a separate local overlay; search prefers it without discarding the source/AI audit relationship. If later recognition conflicts, choose **retain correction**, **accept new result**, or **compare**. Re-indexing never silently overwrites a correction. AI descriptions cannot be cited as verbatim textbook text; check the original page for exact wording or visual detail.
