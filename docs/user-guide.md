# TextbookLens user guide

[English](user-guide.md) · [简体中文](user-guide.zh-CN.md) · [繁體中文](user-guide.zh-TW.md)

TextbookLens is a local-first Windows 11 x64 reader for PDF, EPUB, and DOCX textbooks. Changing the interface language does not translate a book, note, question, or answer.

## Install and start

1. Open the [latest release](https://github.com/gh615280-maker/TextbookLens/releases/latest).
2. Choose the standard EXE for the simplest installation. Use an MSI for managed deployment. Choose a file containing `offline` when the computer has no WebView2 and cannot access the network.
3. Optionally verify the installer with `SHA256SUMS.txt`.
4. Windows may show an unknown-publisher warning because the packages are not code-signed.

The first-run flow begins by choosing a book. AI is optional: use **Auto-connect local AI** for an installed Ollama/LM Studio runtime, configure a supported cloud provider, or continue with local reading features.

## Import and read

1. In **Library**, choose **Import** or drop a PDF, EPUB, or DOCX. TextbookLens copies it into its app-owned local data area, then parses and indexes it.
2. Search, reading position, notes, annotations, and completed history remain available locally.
3. Select text to explain, give examples, derive steps, translate, ask a question, or create a note.
4. Use **Box select** for a formula, chart, illustration, or other region. A visual request shows the provider, model, content type, image count, and risk before its first send.
5. A completed answer becomes a marker and floating panel. **Hide** closes only the panel; **Stop** cancels the active request. Reopen an answer from its marker or history.

## Questions and context

TextbookLens keeps study records by book instead of scattering them across unrelated chat windows. For each question it packs a bounded selection, relevant current-book retrieval, needed same-book history, the current teaching instruction, and the question. It does not send the whole library or remove the AI model's context limit.

A book's **Learning overview** is local. Book-level questions become available after full-text preparation and retrieve relevant material from that book only. Normal PDF text layers, EPUB, and DOCX text extraction do not need an AI model.

## Local and cloud AI

- **Ollama:** one-action discovery can start the local service, find installed text models, test one with a synthetic question, and register usable profiles. Compatible vision models can be selected separately.
- **LM Studio:** local text integration is experimental; real-machine validation is deferred to v0.2.1. Vision is not promised.
- **Cloud providers:** OpenAI, Gemini, Anthropic, DeepSeek, and Kimi require a user-supplied API key. Keys are stored through Windows Credential Manager.
- TextbookLens does not install runtimes, download models, include model weights, or silently fall back from local to cloud AI.

See [AI services](ai-services.md) and [Offline local models](local-models.md).

## What stays local and what can be sent

| Data or action                                                    | Local behavior                                   | Sent after an explicit action? |
| ----------------------------------------------------------------- | ------------------------------------------------ | ------------------------------ |
| Imported copy, normalized sections, local index, reading position | App-owned files and SQLite                       | No                             |
| Reading, search, notes, markers, completed history, language      | Local and available offline                      | No                             |
| Settings, teaching instruction, profile metadata, consent         | Local; cloud keys use Windows Credential Manager | No                             |
| Selected text or follow-up                                        | Frozen selection plus bounded same-book context  | Yes                            |
| Selected region image                                             | Bounded local capture before confirmation        | Yes                            |
| AI-assisted exceptional-page indexing                             | Local detection and validation                   | Yes, confirmed pages only      |
| Kimi Files preparation                                            | Result and local index stay local                | Yes, selected PDF only         |
| Book-level question                                               | Relevant current-book retrieval                  | Yes                            |

Declining a confirmation sends nothing. A remembered choice is limited to that profile and content category; it cannot authorize background uploads.

## Backup, deletion, and recovery

**Create backup** writes a versioned `.tlbackup` containing app-owned books and local study data, but no API keys, logs, temporary captures, absolute source paths, or unfinished AI output. **Restore backup** validates version, paths, sizes, hashes, and SQLite integrity before scheduling an atomic restart swap.

**Remove from library** deletes one app-owned copy and its related local records, not the original source. **Clear all data** requires exact confirmation and removes TextbookLens local data and credentials; separately saved backups and original books are not targets. Uninstall is not a data-erasure guarantee. See [Data, backup, restore, and deletion](data-backup.md).

## Current limitations

- Windows 11 x64 only; packages are unsigned and there is no automatic updater.
- Local whole-book OCR and local whole-book visual indexing are not implemented.
- Local vision can be wrong on dense charts, handwriting, or complex formulas.
- Large scanned PDFs may require manual zoom adjustment.
- Current provider availability, quotas, prices, retention, and answer quality are controlled by each provider/model, not TextbookLens.
