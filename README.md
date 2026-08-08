# TextbookLens

TextbookLens is a Windows 11 x64, local-first textbook reader for PDF, EPUB, and DOCX. It keeps its authoritative library and learning data on the device. AI is optional: content is sent only for an action that you explicitly start.

## Start here

- [User guide](docs/user-guide.md) — import, reading, AI actions, history, and deletion, with essential instructions in 简体中文 / 繁體中文 / English.
- [AI services and sending boundary](docs/ai-services.md) — providers, capability gates, Kimi regional detection, and sent-data boundaries.
- [Data, backup, restore, and deletion](docs/data-backup.md) and [Index quality](docs/index-quality.md).
- [Contributing](CONTRIBUTING.md), [security reporting](SECURITY.md), and [third-party notices](THIRD_PARTY_NOTICES.md).

## Product boundary

- Import, reading, local search, annotations, language changes, backup, and restore do not require an AI request.
- API keys are held by Windows Credential Manager, not SQLite, frontend storage, logs, diagnostics, or backups.
- The application has no account, cloud sync, collaboration service, web/mobile client, project proxy, or bundled model credit.

The current implementation is not a release approval. Task 1–5 automated work is complete; Narrator/NVDA, physical Windows DPI/contrast, installer, clean Windows, real-provider, signing, packaging, and publishing gates remain **NOT RUN**. See the [executable acceptance standard](docs/testing/2026-08-08-textbooklens-v1-executable-acceptance-standard.md).

## Development

Use the exact commands and scope rules in [CONTRIBUTING.md](CONTRIBUTING.md). The project is declared Apache-2.0 in its package manifests; the dependency and fixture review record is in [LICENSES.md](LICENSES.md).
