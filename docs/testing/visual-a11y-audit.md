# TextbookLens V1 trilingual, accessibility, Fluent, and responsive audit

Date: 2026-08-08
Phase: 15, Task 4
Base: `419822487b36b9a77454a9991f6e9667ee929bff`
Status: automated Task 4 gates PASS; physical assistive-technology and Windows display gates remain NOT RUN as listed below

## Result

The V1 UI is release-audited for zh-CN, zh-TW, and en under a synthetic, local-only harness. Automated axe scans reported zero violations on the seven standard primary routes and the reader route. Keyboard skip navigation, modal focus containment and return, forced-colors opaque rendering, reduced motion, the 320/480/768/1024/1440 viewport matrix, logical 125/150/200 percent display-scale proxies, and pseudo-long-string overflow checks pass.

This audit does not claim a Narrator, NVDA, physical multi-display, real Windows display-scale, Mica, installed-package, or clean-Windows pass. Narrator is present on this machine but was not started. NVDA was not found in the standard installation paths or uninstall registry. Those items remain explicit manual gates.

## Safety and fixture boundary

- All browser execution uses self-made identifiers, titles, dates, HTML, and counts from `e2e/accessibility-visual.spec.ts`.
- The Tauri boundary rejects every unrecognized IPC command. No provider command, credential, textbook source, personal data, or production profile is available to the harness.
- Browser requests are allowed only for `127.0.0.1`, `localhost`, `data:`, and `blob:`. Any other request is aborted and recorded; the final assertion requires an empty record.
- No real provider, external network, user textbook, screenshot, credential, desktop shortcut, installed app process, or P9B build target was touched.
- Task 3 privacy and supply-chain behavior remains unchanged: this task does not relax redaction, scanning, TLS, capabilities, migrations, backups, completion truth, or credential boundaries.

## Trilingual and Fluent audit

The catalog audit covers 422 keys in each of `zh-CN`, `zh-TW`, and `en`.

- Key sets are exact and every value is nonempty.
- Placeholder sets match for every key in all three catalogs.
- A regression rejects values copied unchanged across all three languages except the three language names and the technical token `LaTeX`.
- The previous English copies in the simplified- and traditional-Chinese AI-assisted indexing and index-quality catalogs were replaced with real translations.
- Active onboarding, import/drop, index-quality review, floating-panel/history, overlapping-marker, and route-gate text now resolves through the catalog.
- A TypeScript JSX audit found only the TextbookLens brand plus two unrouted legacy components (`BookCard` and `ReadingSettings`). Those legacy components are reachable only from their unit tests, not from a V1 route; they are not represented as audited release surfaces.
- Provider/profile/model identifiers, book titles, user questions, selections, and backend-safe errors remain data, not translated UI copy.

The live route inventory is:

| Surface                               | zh-CN / zh-TW / en         | Automated route/axe               | Notes                                                                                      |
| ------------------------------------- | -------------------------- | --------------------------------- | ------------------------------------------------------------------------------------------ |
| `/onboarding`                         | PASS                       | PASS                              | Book, provider, visual-text, ready, loading, retry, and import copy cataloged              |
| `/library`                            | PASS                       | PASS                              | Command bar, context menu, remove dialog, drop overlay, progress, empty/error states       |
| `/teaching-instructions`              | PASS                       | PASS                              | Editor, presets, conflict, temporary test, replace/leave dialogs                           |
| `/ai-services`                        | PASS                       | PASS                              | Provider, key replacement/deletion, consent reset; no credential/region internals rendered |
| `/books/:bookId/overview`             | PASS                       | PASS                              | Local overview, questions, history, confirmation/delete dialogs                            |
| `/settings`                           | PASS                       | PASS                              | Reader settings, maintenance, backup/restore/clear dialogs and localized live status name  |
| `/books/:bookId/index-quality/:runId` | PASS                       | PASS                              | Aggregate/page/run/block/review/error text fully cataloged                                 |
| `/books/:bookId/index-start`          | PASS by focused regression | Covered by existing focused tests | Full-text extraction route                                                                 |
| `/books/:bookId/index-advanced`       | PASS by focused regression | Covered by existing focused tests | Explicit AI-assisted page-index confirmation                                               |
| `/books/:bookId/read`                 | PASS                       | PASS                              | Reader copy is a complete typed three-language record; reader skip link added              |

All primary controls use native HTML or Fluent-compatible CSS. The UI does not require transparency. Surface backgrounds resolve to opaque theme colors and to `Canvas`/`CanvasText` in forced colors. Windows 11 Mica is optional and currently not used; therefore transparency-off and effect-failure behavior is operationally identical to the opaque path.

## Accessibility audit

### Automated PASS

- Named `main`, navigation, toolbar, region, dialog/alertdialog, group, status, and alert landmarks.
- Skip links on standard, onboarding, and reader shells; activation focuses the target main landmark.
- Keyboard-only reader toolbar, search drawer, settings drawer, table of contents, language menu, settings operations, and panel controls.
- Modal initial focus, Tab/Shift+Tab containment, Escape close, and focus return for learning confirmation, index start, teaching replace/leave, library removal, provider deletion/consent, note deletion, overview confirm/delete, settings maintenance, reader drawers, and overlapping-marker selection.
- Floating-panel title handle is focusable and supports Arrow movement; all eight resize handles have localized accessible names and keyboard operation.
- `aria-live` is reserved for bounded status changes. Streaming panel output uses a throttled polite announcement rather than announcing each token.
- Forced-colors active with dark color scheme: opaque backgrounds remain operational, actions stay visible, and axe reports no violation.
- Reduced-motion active: computed animation and transition duration is at most `0.01ms` (Chromium reports this equivalently as `1e-05s`).
- Color is not the only signal for selection, status, progress, errors, marker kinds, or focus; text, shape, native roles, borders, and focus outlines remain present.

### Reproducible Narrator/NVDA checklist — NOT RUN

Run this only against a Task 7/8 release candidate in a disposable Windows profile with synthetic fixtures. Do not use the user-authoritative desktop shortcut build as Task 4 evidence.

1. Start Narrator (`Win+Ctrl+Enter`) or NVDA and open each primary route using only the keyboard.
2. Confirm the window title, single level-one heading, main/navigation landmarks, language menu state, and skip-link target in zh-CN, zh-TW, and en.
3. On the reader, open Contents, Search, and Aa; confirm dialog name, initial focus, Tab/Shift+Tab containment, Escape close, and focus return.
4. Trigger one synthetic learning confirmation and one saved synthetic history panel. Confirm provider/model metadata, bounded live announcements, title-handle name, eight resize-handle names, Stop/Retry/Delete state, and follow-up label.
5. Exercise overview question/history, provider delete/consent reset, teaching replace/leave, index start, library remove, note delete, and settings backup/restore/clear dialogs. Confirm accessible name, description, disabled state, alert/status announcement, and return focus.
6. Confirm selection markers, personal-note markers, overlapping-marker menu, indexing progress/resume, safe errors, and retry actions do not rely on color alone.
7. Repeat with Windows Contrast Themes, animations disabled, and transparency disabled.

Machine discovery on 2026-08-08:

- Narrator executable: `C:\Windows\system32\Narrator.exe`; installed, NOT RUN.
- NVDA: not found in standard Program Files/local-app paths or uninstall registry; NOT RUN, not PASS.

## Responsive and visual audit

| Matrix                               | Automated result             | Limit                                                                                                             |
| ------------------------------------ | ---------------------------- | ----------------------------------------------------------------------------------------------------------------- |
| 320, 480, 768, 1024, 1440 CSS px     | PASS                         | Chromium viewport resize, not a physical monitor switch                                                           |
| 125%, 150%, 200%                     | PASS proxy                   | Tested as 1440 physical px divided to 1152/960/720 logical CSS px; actual Windows scale NOT RUN                   |
| Pseudo-long strings at 320 px        | PASS                         | Visible main headings/controls are expanded with synthetic long copy; no document overflow                        |
| Forced colors + dark scheme          | PASS                         | Chromium forced-colors emulation; real Windows Contrast Theme NOT RUN                                             |
| Reduced motion                       | PASS                         | Chromium media emulation and computed-style assertion                                                             |
| Reader fullscreen                    | PASS mock/focused regression | Browser Fullscreen API is deterministic in the Tauri mock; real maximized/fullscreen app NOT RUN                  |
| Display resize / panel recovery      | PASS unit regression         | A visible panel is reclamped to 320×240 and its title handle remains visible; physical multi-display move NOT RUN |
| Mica/transparency off/effect failure | Opaque path PASS             | Mica is not implemented or required; physical effect toggles NOT RUN                                              |

The index-review grid now collapses without overflow, and local preview images/selectors/editors are constrained to their container. The floating panel listens to both window and visual-viewport resize, recalculates its clamped fixed rectangle, wraps long title-bar actions, and retains title access at the smallest tested viewport.

Recent V1 regressions retained by the full/focused suite include PDF/EPUB/DOCX region selection under CSS zoom, reader/textbook resize and Aa zoom, eight-way floating-panel resize, marker/highlight relocation and scroll restore, overview/book-question/history flows, AI services, teaching instructions, maintenance settings, library AI indexing progress/resume, and Kimi UI boundaries.

## Verification evidence

Task 4 verification evidence:

- `npm.cmd run test:e2e -- e2e/accessibility-visual.spec.ts` — PASS, 4/4 tests.
- Focused Vitest coverage — PASS, 16 files and 51 tests, covering catalogs, reader focus, floating panels, indexing, onboarding/import, library drop behavior, and overlapping markers; the full suite covers the remaining modal owners and route surfaces.
- `npm.cmd test` — PASS, 96 files and 379 tests.
- `npm.cmd run build` — PASS; Vite retained its pre-existing large-chunk advisory.
- `npm.cmd run lint` — PASS.
- `npm.cmd run typecheck` — PASS.
- `npm.cmd run format:check` — PASS.
- `npm.cmd run check:sensitive` — PASS, 586 controlled files.
- `npm.cmd run check:licenses` — PASS, 197 production packages.
- `npm.cmd run check:generated` with `CARGO_TARGET_DIR=D:\CodexBuild\textbooklens-p15t4-target`, `TEMP=TMP=D:\CodexBuild\textbooklens-p15t4-temp`, `CARGO_INCREMENTAL=0`, and `CARGO_BUILD_JOBS=1` — PASS, one binding export test; generated files unchanged. The first cold isolated invocation exceeded the command time limit while compiling and was not counted as a pass; after that process completed naturally, the same gate returned exit code 0 from the warm isolated target.
- Axe — PASS with zero violations on seven standard routes plus the reader route.
- Viewport assertions — PASS at 320/480/768/1024/1440.
- Logical display-scale proxies — PASS at 125/150/200 percent.

## Gates intentionally NOT RUN

- Narrator critical-flow walkthrough.
- NVDA critical-flow walkthrough (NVDA not installed).
- Actual Windows 125/150/200 percent display-scale changes.
- Real maximized/fullscreen, physical monitor switch, and DPI transition.
- Real Windows Contrast Theme, transparency-off, Mica, or composition-effect failure toggles.
- Task 7/8 installer, installed clean-Windows, upgrade/uninstall, real provider, credential, packaging, signing, publishing, and release gates.
