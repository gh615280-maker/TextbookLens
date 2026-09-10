# Data, backup, restore, and deletion

[English](data-backup.md) · [简体中文](data-backup.zh-CN.md) · [繁體中文](data-backup.zh-TW.md)

## Local data contract

The app-owned local data includes imported textbook copies, normalized reading data, local FTS, reading positions, page-index states/results, provenance/correction overlays, completed conversations/messages/citations, notes, annotations, markers, panel preferences, settings, language, teaching instruction, consent choices, and non-sensitive provider metadata. API keys are device-local Windows Credential Manager data, not SQLite data.

## Backup and restore

**Create backup** makes one versioned local `.tlbackup` file at your selected destination. It shows an estimated size and warns that the archive contains app-owned textbook copies; check copyright and sharing permissions before sharing it.

The archive contains a consistent SQLite snapshot, app-owned original textbook copies, necessary derived reading files, completed history, notes/annotations/markers, settings, teaching instruction, indexes, corrections, and provenance. Every entry has a relative path, declared size, and SHA-256 record. It does **not** contain API keys, credential identifiers, logs, diagnostics, unfinished output, temporary images, cache, remote temporary resources, or absolute source paths. It is not cloud sync and TextbookLens does not upload it.

**Restore backup** verifies format/version, sizes, hashes, allowed relative paths, SQLite integrity, and compatibility before preparing an atomic maintenance/restart swap. Failure keeps the current data. Import, indexing, and learning must not be active. After restart, books, indexes, corrections, completed history, notes, and settings are local again; credentials do not transfer, so reconnect AI services if needed.

## Delete one book, clear all, and uninstall

**Remove from library** is a separately confirmed operation. It removes only the app-owned copy of that book and its derived files, indexes, corrections, conversations, notes, markers, and remote-resource tracking. It does not alter your original source, another book, or an external `.tlbackup`. Remote cleanup failure stays visible/retryable and never resurrects local data.

**Clear all data** is not delete-one-book and is not uninstall. It always requires the exact on-screen confirmation. It targets every TextbookLens local item: database, settings, teaching instruction, app-owned books/derived data, indexes/corrections, completed history, notes/annotations/markers, cache, logs, staging/trash, remote-resource records, and all TextbookLens credentials. It records a recoverable intent first. If credential cleanup fails, the app reports retry required and does not report full completion. A successful clear requires restart to return to first-run. Originals, separately saved `.tlbackup` files, and files outside the app-owned root are not targets.

Uninstall is intentionally not a data-erasure guarantee. Version 0.2.0 installer and uninstall smoke tests passed in isolated Windows environments, but use **Clear all data** when the goal is to remove TextbookLens local data and credentials.
