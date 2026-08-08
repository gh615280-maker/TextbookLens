# Destructive local-data boundary guarantees

This note records the release-test contract for destructive local operations. It is test evidence, not a user guide. Every destructive test uses a `TempDir`, generated SQLite data, an in-memory credential boundary, or a project-owned synthetic fixture. It never selects the real application-data directory, a real credential namespace, a user source, or a real provider.

## Guaranteed recovery states

- Database upgrades accept only `user_version = 0` and an exact, successful, contiguous prefix of the embedded migration history. Version, description, checksum, and migration-table shape are checked before any pending migration runs. A fresh database and every historical cutoff from `0001` through `0011` must converge to the same schema, foreign keys, triggers, FTS objects, constraints, and reopen behavior.
- Each migration and its history row commit in one `BEGIN IMMEDIATE` transaction. The two foreign-key-rebuild migrations (`0002` and `0011`) disable foreign-key enforcement only before that transaction, run a foreign-key check before commit, and restore enforcement afterward. Failure preserves the complete prior schema and history; no half-applied migration is eligible for retry.
- Backup, delete-one-book, restore, and clear-all use bounded canonical manifests or journals. Recovery preflights every pending plan before applying any plan. Duplicate path/file ownership across journals, malformed later journals, reparses, hardlinks, identity replacement, unexpected entries, or cross-volume moves fail before an earlier plan can mutate data.
- Delete-one-book reaches either the complete old state or the complete locally deleted state. It removes only the target book's app-owned copy, derived files, index/pages, local history, and local remote-resource rows. The source original, external backup, sibling book, and unrelated remote rows remain. A provider cleanup failure stays as a durable retry record and never resurrects the local book.
- Restore validates archive path, manifest grammar, entry ownership, hashes, size/count limits, the SQLite schema/history/integrity, and every materialized file before scheduling mutation. Restart swap recovery is idempotent and exposes only a complete old or complete new dataset.
- Clear-all requires the exact independent confirmation and targets only the canonical TextbookLens root plus TextbookLens credential accounts discovered from the local profile set. Credential failure remains visible and retryable; local deletion does not report success until credential cleanup has succeeded. External `.tlbackup` files, source originals, and sibling/root decoys remain.
- Startup journal recovery is bounded and idempotent. Delete journals are globally preflighted. Backup/restore/clear recovery performs no provider request. Remote-cleanup startup reconciliation handles only an already-written local success marker; pending or failed remote deletion is not retried implicitly.
- Public errors, `Debug`, DTOs, and normalized evidence expose stable codes and structural state only. Paths, book content/title, provider or profile values, credential data, remote handles, and internal identifiers are not emitted.

## Automated ownership

| Boundary                                                                                                | Primary executable evidence                                                         |
| ------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------- |
| Migration hashes, every cutoff, fresh/reopen, malicious history, atomic failure, Windows lock/read-only | `src-tauri/tests/destructive_boundaries.rs`; `src-tauri/tests/database_contract.rs` |
| Canonical root, path syntax, junction/reparse, hardlink and storage identity                            | `src-tauri/tests/destructive_boundaries.rs`; `maintenance::storage::tests`          |
| Journal grammar, temp/write/rename/parent-sync faults                                                   | `maintenance::journal::tests`                                                       |
| Complete target-book deletion, crash replay, decoys, remote retry                                       | `src-tauri/tests/delete_book_complete.rs`                                           |
| Backup manifest, identity races, write/publish faults                                                   | `maintenance::archive::tests`                                                       |
| Restore validation and every staging/swap/finalization boundary                                         | `maintenance::restore::tests`                                                       |
| Clear confirmation, credential retry, local move/finalization boundaries                                | `maintenance::clear_all::tests`; `src-tauri/tests/local_data_privacy_checkpoint.rs` |
| Cross-operation restart ordering and idempotence                                                        | `src-tauri/tests/interrupted_operations.rs`                                         |

## Remaining Windows lock limitation

Windows can deny open, rename, delete, or directory synchronization while another process holds a non-sharing handle, while security software transiently inspects a file, or when ACL/read-only/full-disk conditions prevent progress. TextbookLens does not bypass that handle, terminate the owning process, weaken identity/containment checks, or report success. The operation returns a stable safe error and retains either the unchanged old state or its bounded journal/staging/trash state for an explicit retry or the next bounded startup recovery.

The integration suite uses a real Win32 non-sharing file handle and a read-only database to reproduce this rule. Rename, write, temporary-file, database-commit, and parent-sync denial points are additionally injected deterministically by the maintenance fault matrices. Once the external lock or storage condition is removed, the same synthetic operation can be retried; recovery does not spin, broaden its targets, or silently issue a provider request.
