# 0002: Local data and credential boundary

## Status

Accepted on 2026-08-02.

## Decision

TextbookLens owns one application-data directory selected by Tauri. Rust creates
and manages `library.sqlite3`, `books/`, `cache/`, and `logs/` below that root.
Only Rust may open or mutate SQLite. The frontend obtains narrow DTOs through
Tauri commands and never receives database paths or direct database access.

An imported source is copied into `books/` before parsing. Import cancellation,
failure, retry, and later deletion operate on the application-owned copy. The
file at the user's original import location is never changed or deleted.

API credentials have one temporary exception to the frontend boundary: a value
may exist in a password field and its save-command IPC arguments while the user
enters it. After saving, the frontend retains only a provider profile UUID.
Plaintext credentials are never written to SQLite, browser storage, logs,
diagnostics, fixtures, commits, or AI events.

Rust stores credentials in Windows Credential Manager through keyring service
`TextbookLens`; the account name is `textbooklens/{profile_uuid}`. SQLite stores
that account reference in `provider_profiles.credential_key`, never the secret.
Replacing a credential must validate the candidate before changing a working
credential. Deletion removes the credential and profile as one user-visible
operation; database failure triggers restoration of the prior credential.

AI requests travel directly from Rust to the selected provider's official
endpoint. They include only metadata and content required for the active
request. There is no project server, telemetry, analytics, or automatic crash
upload.

Logs and diagnostics exclude authorization headers, credentials, credential
command arguments, request/response bodies, selected text, book block text, and
complete AI contexts. Diagnostics use normalized error codes, random diagnostic
IDs, bounded redacted details, application version, OS metadata, and operation
stage only.

## Metadata-only data flow

```text
Native file choice -> Rust import lifecycle -> App-owned source metadata + SQLite metadata
Password entry -> One save IPC call -> Rust credential interface -> Windows Credential Manager
Provider profile UUID -> Rust profile lookup -> Credential account reference -> Official provider
Normalized error code -> Redacting diagnostics layer -> App-owned rolling log files
```

## Deletion boundaries

- Provider deletion removes its Windows credential and SQLite profile. If it was
  active, Rust selects the oldest remaining profile or clears the active profile.
- Credential deletion failure preserves the SQLite row.
- SQLite deletion failure restores the credential; if restoration also fails,
  subsequent profile listing reports the credential as missing.
- Book deletion, implemented in a later phase, may remove only the app-owned
  source copy and its related database rows and derived cache entries.
