# TextbookLens

TextbookLens is a Windows 11 x64, local-first textbook reader for PDF, EPUB, and DOCX. It keeps its authoritative library and learning data on the device. AI is optional: content is sent only for an action that you explicitly start.

## Download for Windows

Open the [latest release](https://github.com/gh615280-maker/TextbookLens/releases/latest) and download `TextbookLens_*_x64-setup.exe`. The MSI package is also available for administrators and managed installation.

The current packages are unsigned. Windows SmartScreen may show an unknown-publisher warning; verify the file against `SHA256SUMS.txt` on the release page before choosing **Run anyway**. TextbookLens currently supports Windows 11 x64 only.

## Start here

- [User guide](docs/user-guide.md) — import, reading, AI actions, history, and deletion, with essential instructions in 简体中文 / 繁體中文 / English.
- [AI services and sending boundary](docs/ai-services.md) — providers, capability gates, Kimi regional detection, and sent-data boundaries.
- [Data, backup, restore, and deletion](docs/data-backup.md) and [Index quality](docs/index-quality.md).
- [Contributing](CONTRIBUTING.md), [security reporting](SECURITY.md), and [third-party notices](THIRD_PARTY_NOTICES.md).

## Product boundary

- Import, reading, local search, annotations, language changes, backup, and restore do not require an AI request.
- API keys are held by Windows Credential Manager, not SQLite, frontend storage, logs, diagnostics, or backups.
- The application has no account, cloud sync, collaboration service, web/mobile client, project proxy, or bundled model credit.

Version 0.1.0 passed the local unsigned V1 release gate with documented exceptions. Code signing, timestamping, automatic updates, and several manual accessibility/provider checks are not complete. See the [release checklist](docs/release-checklist.md) and [executable acceptance standard](docs/testing/2026-08-08-textbooklens-v1-executable-acceptance-standard.md) for the exact evidence and limitations.

## Development and modification

The complete source is available under Apache-2.0. Clone and bootstrap the pinned toolchain:

```powershell
git clone https://github.com/gh615280-maker/TextbookLens.git
cd TextbookLens
npm ci
npm run tauri dev
```

Development requires Node.js 24.18.1, npm 11.16.0, Rust 1.97.1, and the Windows desktop build prerequisites used by Tauri 2. Run `npm run preflight` for the release-oriented validation suite and `npm run tauri build` to create local NSIS/MSI installers.

Use the exact contribution rules in [CONTRIBUTING.md](CONTRIBUTING.md). Dependency and fixture review records are in [LICENSES.md](LICENSES.md), with bundled notices in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
