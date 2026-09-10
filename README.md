<div align="center">

![TextbookLens — local-first textbook reader](docs/assets/textbooklens-social-preview.jpg)

# TextbookLens

**Read, search, annotate, and ask questions about PDF, EPUB, and DOCX textbooks in one private Windows app.**

[![CI](https://github.com/gh615280-maker/TextbookLens/actions/workflows/ci.yml/badge.svg)](https://github.com/gh615280-maker/TextbookLens/actions/workflows/ci.yml)
[![Latest release](https://img.shields.io/github/v/release/gh615280-maker/TextbookLens?display_name=tag&sort=semver)](https://github.com/gh615280-maker/TextbookLens/releases/latest)
[![Downloads](https://img.shields.io/github/downloads/gh615280-maker/TextbookLens/total)](https://github.com/gh615280-maker/TextbookLens/releases)
[![License](https://img.shields.io/github/license/gh615280-maker/TextbookLens)](LICENSE)
[![Windows 11](https://img.shields.io/badge/Windows_11-x64-0078D4?logo=windows11)](https://github.com/gh615280-maker/TextbookLens/releases/latest)

[**Download for Windows**](https://github.com/gh615280-maker/TextbookLens/releases/latest) · [English](README.md) · [简体中文](README.zh-CN.md) · [繁體中文](README.zh-TW.md)

[User guide](docs/user-guide.md) · [AI services](docs/ai-services.md) · [Local models](docs/local-models.md) · [Roadmap](ROADMAP.md)

</div>

## Why TextbookLens?

- **One focused library:** import PDF, EPUB, and DOCX books without juggling separate readers.
- **Persistent study context:** your books, reading position, notes, annotations, and completed question history stay organized by textbook.
- **Less chat fragmentation:** ordinary AI chats have finite context, so long study sessions often spread across new conversations and repeated uploads. TextbookLens retrieves relevant current-book material for each question. It does not remove a model's context limit, but it reduces repeated setup and keeps the learning trail together.
- **Local-first by design:** reading, local search, annotations, history, and backups remain on your device.
- **AI only when you ask:** optional AI operations start only from an explicit action and send the bounded content needed for that operation.
- **Offline local AI:** connect installed Ollama text models with one action. Compatible Ollama vision models can answer questions about a selected image region. LM Studio text support remains experimental.
- **Open and modifiable:** the Rust, React, Tauri, and SQLite source is available under Apache-2.0.

<div align="center">

![TextbookLens library interface with synthetic test data](docs/assets/textbooklens-library.png)

<sub>Actual application interface shown with synthetic test data.</sub>

</div>

## Download and use

Open the [latest release](https://github.com/gh615280-maker/TextbookLens/releases/latest) and choose:

- `TextbookLens_*_x64-setup.exe` — recommended small installer; uses the existing WebView2 runtime or downloads it when missing.
- `TextbookLens_*_x64_en-US.msi` — standard MSI for managed installation.
- A file containing `offline` — complete offline installer with the Microsoft WebView2 runtime included.
- `SHA256SUMS.txt` — checksums for all four installers.

Neither package type includes Ollama, LM Studio, or model weights. After installation, import a PDF, EPUB, or DOCX. AI is optional: connect an installed local runtime or configure a supported cloud provider only when needed.

> [!IMPORTANT]
> Current packages support Windows 11 x64 and are not code-signed. Windows SmartScreen may show an unknown-publisher warning. Verify the SHA-256 before choosing **Run anyway**.

## Privacy boundary

TextbookLens has no account system, cloud sync, bundled AI credit, project proxy, collaboration service, telemetry, or silent background upload. Cloud API keys are stored through Windows Credential Manager rather than the application database, browser storage, logs, diagnostics, or backups. Local Ollama and LM Studio profiles do not require a cloud key.

Importing, reading, local search, annotations, language changes, backup, and restore do not require an AI request. See [AI services and sending boundary](docs/ai-services.md), [Offline local models](docs/local-models.md), and [Data, backup, restore, and deletion](docs/data-backup.md) for exact behavior.

## For developers

```powershell
git clone https://github.com/gh615280-maker/TextbookLens.git
cd TextbookLens
npm ci
npm run tauri dev
```

The pinned toolchain is Node.js 24.18.1, npm 11.16.0, Rust 1.97.1, and the Windows prerequisites for Tauri 2. Run `npm run preflight` for release-oriented validation and `npm run tauri build` for local NSIS/MSI packages.

Read [CONTRIBUTING.md](CONTRIBUTING.md) before changing data, credentials, networking, migrations, fixtures, or generated bindings. Dependency provenance is recorded in [LICENSES.md](LICENSES.md) and [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).

## Project status

Version 0.2.0 is the current early public release. It adds one-action Ollama discovery, offline text Q&A, independent Ollama vision selection, bundled PDF decoding resources, and reader reliability fixes. The release passed the repository's frontend, Rust, security, Windows build, installer, offline-install, and v0.1.0 upgrade checks documented in the [v0.2.0 validation record](docs/releases/v0.2.0-validation.md).

Known limits remain: packages are unsigned, automatic updates are not implemented, LM Studio real-machine text validation is deferred to v0.2.1, local whole-book OCR/vision indexing is not implemented, and local vision answers can be factually wrong on difficult material. See the [v0.2.0 release notes](docs/releases/v0.2.0.md).

Ideas and bug reports are welcome through [GitHub Issues](https://github.com/gh615280-maker/TextbookLens/issues). If the project is useful, **star the repository** so more readers and contributors can find it.
