# Windows release checklist

This checklist is the technical record for a **local, unsigned Windows 11 x64 V1 build**. It is
not a release approval. The cross-provider credential-retention/routing defect and the later
visual-learning capture/anchor defect were fixed in the replacement release candidate. A continued
Task 8 run on that candidate passed the credential-isolation regression, three-format full-reader
restart, DeepSeek text, and Kimi vision gates. Its technical ledger remains incomplete: OpenAI,
Gemini, and Anthropic have no test credentials, and required structured/provider, true 125/150/200%
system scaling, backup/restore, delete/clear, fullscreen, and live-region coverage remains NOT RUN or
PARTIAL. On 2026-08-09 the user explicitly accepted those listed V1 release risks. The Task 8 release
decision is therefore **ACCEPTED WITH DOCUMENTED EXCEPTIONS**: this is not an unconditional technical
PASS, does not alter any gate status, and is not a permanent feature guarantee. Windows 10 is neither
supported nor validated. Task 9 is permitted to begin in a separate task but remains **NOT RUN** here.

Task8 ACCEPTED WITH DOCUMENTED EXCEPTIONS; Task9 NOT RUN.

## Immutable inputs

| Input                  | Required value                                                                                                                                                                                           |
| ---------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Source commit          | `8d0b83257f8ded7236e5feb6cd38d4cb16f13f27` (visual capture/anchor fix; parent RC record `23a88e38b81e50c7d81c4321e69decd02888e6dc`; credential isolation fix `4d37a5b3de7e096d9241686a2c85bd0bf3b3c5d3`) |
| Node / npm             | `v24.18.1` / `11.16.0`                                                                                                                                                                                   |
| Rust / Cargo           | `1.97.1` / `1.97.1`                                                                                                                                                                                      |
| Rust target            | `x86_64-pc-windows-msvc`                                                                                                                                                                                 |
| Tauri CLI / Rust crate | `2.11.4` / `2.11.5`                                                                                                                                                                                      |
| Application version    | `0.1.0` (`package.json`, `Cargo.toml`, and `tauri.conf.json`)                                                                                                                                            |
| Product / identifier   | `TextbookLens` / `dev.textbooklens.desktop`                                                                                                                                                              |
| Dependency records     | `package-lock.json` SHA-256 `E7593397180DCE443149265BF569464823CD43CB5650C586BB05003CD64776B0`; `src-tauri/Cargo.lock` SHA-256 `5AFE9A8782AEB56BC9A90F0FD3821DC48249DA8035CFEE818DB527259463E2EE`        |
| Toolchain record       | `rust-toolchain.toml` SHA-256 `6B5C36CC63BE7BF3A075574039B8A49C1361FC1C3ACFCE234AFD28BCC7DECF13`                                                                                                         |

No lockfile may change during installation, checking, or bundling. The release build uses
`CARGO_INCREMENTAL=0`, `CARGO_BUILD_JOBS=1`, Cargo `-j 1`, a task-scoped target directory, and a
task-scoped `TEMP`/`TMP` directory.

## Clean-source procedure

Use a Git-byte-preserving local clone/snapshot; do not use a checkout with `core.autocrlf` enabled.
The clone keeps the Git index required by the sensitive-file guard. On this project, CRLF conversion
changes SQLx migration checksums. Before each build, verify every migration hash, including
`0012`–`0015`, against the Git blob bytes.

```powershell
$env:TEMP = 'D:\CodexBuild\textbooklens-p15t7-visualfix-20260809-01-temp'
$env:TMP = $env:TEMP
$env:CARGO_INCREMENTAL = '0'
$env:CARGO_BUILD_JOBS = '1'
git clone --no-local -c core.autocrlf=false . D:\CodexBuild\textbooklens-p15t7-visualfix-20260809-01-source
Set-Location D:\CodexBuild\textbooklens-p15t7-visualfix-20260809-01-source
npm.cmd ci --offline
cargo.exe metadata --locked --manifest-path src-tauri/Cargo.toml --no-deps
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/preflight.ps1 -Stage All -BuildRoot D:\CodexBuild\textbooklens-p15t7-visualfix-20260809-01-build -Offline
```

When a network-resolution/download phase is authorized, it and the `npm ci --offline` cache-only
phase must be recorded separately. This refresh authorized no network phase: a verified prior cache
was copied into a fresh task-only npm-cache path, and `npm ci --offline` installed 705 packages with
0 vulnerabilities. A missing cache is **NOT RUN**, never an offline pass. Do not read or write normal
user app data, Documents, or normal Temp during the clean-build procedure.

## Required automated gates

```powershell
npm.cmd run format:check
npm.cmd run lint
npm.cmd run typecheck
npm.cmd run test
npm.cmd run build
cargo.exe fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo.exe clippy --locked --manifest-path src-tauri/Cargo.toml --all-targets --all-features -j 1 -- -D warnings
cargo.exe test --locked --manifest-path src-tauri/Cargo.toml --all-features -j 1
npm.cmd run check:generated
npm.cmd run check:sensitive
npm.cmd run check:licenses
npm.cmd run fixtures:verify
node.exe scripts/audit-release-artifacts.mjs --no-defaults --root <absolute-bundle-root> --root <absolute-resources-root> --root <absolute-migrations-root> --file <absolute-LICENSES.md> --file <absolute-THIRD_PARTY_NOTICES.md>
```

`scripts/preflight.ps1` exposes the same gates by named stage (`Toolchain`, `Frontend`, `Rust`,
`Integrity`, `Acceptance`, `Bundle`, or `All`). Its acceptance subset is A, B, O, P, and Q, runs
with one worker, and uses only the existing synthetic loopback/fixture test boundary.

## Package policy and artifact evidence

The bundle must contain only Tauri runtime output, the built frontend, the allowlisted provider
registry, migrations, notices, and configured icon. It must not contain source maps, logs, test
fixtures, credentials, user paths/data, debug symbols, or a development-server dependency. The
release executable's only development-URL string hit is the Tauri `devUrl` configuration literal
`http://localhost:1420`; release packaging uses `frontendDist` and ships no dev server. The final
artifact directory is `D:\CodexBuild\textbooklens-p15t7-release`; it is retained for Task 8.

| Artifact                            | SHA-256                                                            | Size (bytes) | Scanner / inspection                                                                                                           |
| ----------------------------------- | ------------------------------------------------------------------ | -----------: | ------------------------------------------------------------------------------------------------------------------------------ |
| NSIS installer                      | `925E93FAD440715AC7D9A495571F9E745279B157073A4B61F84525CF609F9BDE` |    9,001,336 | Scanner PASS; outer bootstrap PE `0x014c`; release payload EXE is x86-64                                                       |
| MSI installer                       | `A50AF90362DB868646D9BEEC32F311C47A9DFD2FD8B8561627415804831ED0E5` |   12,050,432 | Scanner PASS; Product `TextbookLens` `0.1.0`; summary `x64;0`; `ALLUSERS=1`                                                    |
| bundle/resources/migrations/notices | 20 files / 21,108,082 bytes                                        |   21,108,082 | Scanner PASS; no `.map`, `.pdb`, `.log`, fixture, private sentinel, credential, user path/data, or development-server artifact |

Two isolated builds must compare file manifests and SHA-256 values. MSI/NSIS container timestamps,
PE metadata, and a future code signature may make bytes differ; that is an explainable boundary,
not bit reproducibility. Task 9 must repeat the comparison, retain any manifest-level differences,
and must not claim byte-identical installers unless demonstrated.

Task 7 ran two isolated, offline builds from the same locked inputs. Both emitted the same two-file
bundle manifest (`NSIS`, `MSI`) and both artifact scans passed. The containers are not bit
reproducible: build A versus B differs for NSIS (9,000,265 / 8,999,366 bytes; SHA-256
`E77AB169…38DC` / `AAE155DE…3EBF`) and MSI (same 12,046,336-byte length; SHA-256
`938AFFB1…A5DE` / `7E5F6334…486F`). This is recorded as an installer-tool metadata boundary;
Task 9 must retain this result and must not claim byte-identical installation media.

The visual-fix refresh ran the full offline `Stage All` preflight from clean commit
`8d0b83257f8ded7236e5feb6cd38d4cb16f13f27`: frontend format/lint/typecheck/build, 96 Vitest files
and 383 tests, Rust fmt/clippy/all tests (477 passed, 1 manual smoke ignored),
sensitive/licenses/fixtures/generated checks, A/B/O/P/Q, both bundles, and the release scanner
passed. The superseded `23a88e3` RC packages were copied byte for byte to read-only
`D:\CodexBuild\textbooklens-p15t7-release-pre-visual-fix-23a88e3-20260809` before the two
authoritative files were replaced. Signing, timestamping, updater configuration, publication, and
remote CI remain **NOT CONFIGURED / NOT RUN**. Task 8 continued with the partial results below; Task 9
remains **NOT RUN**.

## Task 8 clean-Windows and real-provider result

The historical Task 8 run started from exact HEAD `8af9882e44be33e18c91a39e81f324cf98b3b6d2` with parent
`c7716affae03fe820f76aceec34e0055c8105e53` and preserved its initial evidence in
`458ea4f7dc939b070dc58a71290b2a069e601ed2`; the pre-restart continuation was recorded in
`bbe4e2ede0616cd7de210ad5d56feb7d6a4901f5`. The protected Task 7 chain was not rewritten. Start,
environment-switch, and final checks found its then-current Task 7 release directory unchanged.
That execution evidence belongs to the superseded candidate and is retained only as defect history.

The visual-fix refresh initially did not operate the retained Sandbox. It rebuilt the daily-user
desktop target from the same final clean source and launched it once through the unchanged shortcut.
The authoritative debug executable is 43,692,544 bytes with SHA-256
`EEF2D354058422426E5B65851785D0E9B112BBE62D516CF74F6911E776153C33`; the shortcut still targets
`D:\CodexBuild\textbooklens-p9b-target\debug\textbooklens.exe`, loaded the TextbookLens shell with
no localhost/network-error surface, and was closed by its exact new PID. The shortcut itself remains
1,380 bytes with SHA-256 `A5E428D50138B7751DA342370DC12AE73076453395B39031490A3B2F3E5A1905`.

Task 8 later resumed the sole retained disposable credential session specifically to preserve the
user-entered DeepSeek/Kimi test credentials while performing a same-version upgrade to the
replacement NSIS. This continuation is valid for upgrade, credential-isolation, app-owned-data, and
provider regression evidence; it is not represented as a new no-state install. Fresh NSIS and MSI
package-entry evidence remains separately established by the earlier disposable sessions. There is
no unresolved product FAIL on the current candidate. Task 8 is **ACCEPTED WITH DOCUMENTED
EXCEPTIONS**, and Task 9 remains **NOT RUN** in this task.

### Environment proof and decision

The daily host is Windows 11 Pro 23H2 build `22631.2861`, x64. It remains excluded from clean
installation, destructive, display, theme, and assistive-technology testing. After the authorized
Microsoft Sandbox feature enablement and normal restart, the host reported an active hypervisor and
a runnable Windows Sandbox feature.

The no-credential sessions used a network-enabled `.wsb` profile with the Task 7 release and
self-made input mapped read-only, a separate writable external-test directory, and clipboard
disabled. A third fresh session used the same minimal mappings with clipboard enabled only for
direct user entry into the product password field:

| Evidence ID         | Guest proof                                                    | Initial scale record                                                     | Lifetime / isolation                                                                                                                                            |
| ------------------- | -------------------------------------------------------------- | ------------------------------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `P15T8-SBX-NSIS-01` | Windows 11 Enterprise 22H2 build `22621.2861`, x64, 4 GiB      | Exact percentage was not surfaced; required 125/150/200% changes NOT RUN | Fresh Sandbox; guest instance `bda89b1d-9684-4cc2-9e17-c4c58c79c059`; destroyed after supplemental no-credential gates                                          |
| `P15T8-SBX-MSI-02`  | Independent fresh launch of the same Enterprise/x64 base image | Exact percentage was not surfaced; no system-scale claim                 | Fresh Sandbox with no inherited app data or credentials; no reusable snapshot exists; destroyed after MSI uninstall                                             |
| `P15T8-SBX-CRED-03` | Independent fresh launch of the same Enterprise/x64 base image | Exact percentage was not surfaced; no system-scale claim                 | Fresh instance `f7f9cf40-d164-4776-bd63-eb020f99627e`; retained for same-version credential/visual-fix RC upgrade and still active at final evidence checkpoint |

Only self-made tiny PDF/EPUB/DOCX/PNG fixtures were exposed to the guest. The primary fixture hashes
were PDF `464847140CCA555A80FF51A0EEBA1072A06175A57239FBF0BE942DAA40B7DF62`, EPUB
`F791BAFA42D089C0081536C53C1B69701DE4019FD6B1EEEECACFA931A5338286`, DOCX
`8017A4B9BEE398496B2F0C8E99084BADF5644960020C57548A18A079E3164595`, and PNG
`6C65EFAA2EBBB9912BA372076E088471EC1F6BF29ED62613C9D30A42569A50EF`. No real textbook, user
database, private account path, or host credential was copied into the recorded sessions. The user
later entered only DeepSeek and Kimi test credentials directly into product password fields. The
agent never read clipboard data, password fields, Credential Manager, disk, environment, or raw Key
material.

The historical cross-provider defect was reproduced on a superseded candidate, then fixed and passed
targeted regression on the replacement candidate; no unresolved product FAIL remains. Multiple
required manual rows remain NOT RUN, PARTIAL, or WARN. The user accepted those specifically listed
V1 risks without changing the test results, so the release decision is **ACCEPTED WITH DOCUMENTED
EXCEPTIONS**. Historical automation and browser emulation remain separate from current manual
evidence.

### Installer execution and signature state

| Input | Version / architecture evidence                                                                                                                   | Signature state                     | Current clean-Windows execution                                                                                                                                                                                                                                                                                                                                                                                                                              |
| ----- | ------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| NSIS  | File/product version `0.1.0`; outer bootstrap PE `0x014c` (x86); Task 7 payload target remains x86-64. The outer stub alone is not payload proof. | `NotSigned`; no signer or timestamp | Primary networked flow PASS: clean install, first launch, PDF onboarding import, restart, same-version `Upgrade install`, relaunch, and uninstall. The retained credential session then completed a same-version upgrade to the replacement credential/visual-fix RC; installed app-owned data and DeepSeek/Kimi connection profiles remained available. A separate offline attempt stopped at the Microsoft WebView2 dependency and is not an offline PASS. |
| MSI   | Product `TextbookLens` `0.1.0`; summary `x64;0`; 64-bit main component; default `C:\Program Files\TextbookLens\`.                                 | `NotSigned`; no signer or timestamp | Independent fresh-session smoke PASS: install, completion launch into first-run onboarding, maintenance-mode Remove, completion, and desktop-shortcut removal. No version-to-version upgrade claim was made.                                                                                                                                                                                                                                                 |

Task 7's matching-content but non-bit-reproducible installer-container result remains unchanged.
Signing, timestamping, updater configuration, publication, and remote CI remain **NOT CONFIGURED /
NOT RUN**.

### Manual gate ledger

| Gate                                                                                  | Status         | Current manual evidence                                                                                                                                                                                                                                                                                                       |
| ------------------------------------------------------------------------------------- | -------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| NSIS primary install/launch/reinstall/uninstall plus MSI independent smoke            | PASS           | Both package entry points ran in separate fresh Sandbox sessions; NSIS same-version upgrade and MSI maintenance removal completed.                                                                                                                                                                                            |
| First run and self-made PDF import/restart/source deletion                            | PASS           | The app-owned PDF copy opened in the full reader after restart and remained available after deletion of only the scoped writable source duplicate. The host original and unrelated files were not touched.                                                                                                                    |
| Self-made EPUB/DOCX import and full three-format reader restart                       | PASS           | Both read-only-mapped self-made files imported, opened in the full reader, displayed their fixture markers (`EPUB-T8-GAMMA` and `DOCX-T8-ALPHA`), and reopened after an app-only restart. Their mapped originals were not deleted, so no extra per-format source-deletion claim is made.                                      |
| `zh-CN` / `zh-TW` / `en` switching and restart persistence                            | PASS           | All three UI languages were selected in the installed NSIS candidate; English persisted after restart.                                                                                                                                                                                                                        |
| True 125/150/200% system scaling                                                      | NOT RUN        | Sandbox Settings did not expose a supported scale control; no browser zoom, emulation, or registry proxy was substituted.                                                                                                                                                                                                     |
| Sandbox display/window resolution change                                              | PASS           | The guest viewport changed from approximately 1353×809 to 1708×1053 and the installed onboarding surface reflowed without a horizontal scrollbar.                                                                                                                                                                             |
| Maximized and narrow readability                                                      | PASS           | Installed onboarding surface remained readable maximized and in an approximately 810-pixel snapped width.                                                                                                                                                                                                                     |
| Fullscreen/F11                                                                        | NOT RUN        | No fullscreen PASS is claimed.                                                                                                                                                                                                                                                                                                |
| Windows Contrast Theme, transparency off, opaque fallback, reduced motion             | PASS (surface) | Aquatic Contrast Theme, transparency off, and animation effects off remained functional and readable on the installed onboarding surface; settings were restored/disposed.                                                                                                                                                    |
| Keyboard-only skip/focus/provider/model/Esc-return path                               | PASS (surface) | Skip link, provider/required-Key/model controls, disclosure, disabled-button skip, and Esc focus return worked. Live-region validation remains NOT RUN.                                                                                                                                                                       |
| Built-in Narrator                                                                     | PASS (surface) | Enabled through Windows Settings. Visible focus traversed provider, required Key, disclosure, and model; skip Enter focused main and Esc returned to provider. Narrator was then disabled. No speech transcript was retained.                                                                                                 |
| NVDA                                                                                  | PASS (surface) | Official NV Access 2026.1.1 binary (62,914,952 bytes, SHA-256 `6E0289EB5A3AA076EB97EA99C5D5465CB48B5ECC6A3257DC3D811F881A1747C9`) matched the published hash and a valid NV Access Limited signature. A temporary run announced only safe onboarding labels/state and the skip link; no log or provider request was retained. |
| Backup to external test directory and restore to a second clean profile/session       | NOT RUN        | The live contract surface reported 776 KB, inclusion of app-owned textbook copies, and exclusion of API Keys. Two safe save-picker routes created no `.tlbackup` in the writable mapped directory; the dialog was canceled and no restore/no-Key reconnect was claimed.                                                       |
| Delete one book, clear all, second-book/original/backup retention, credential cleanup | NOT RUN        | Backup/restore did not complete, so the required ordering prohibited destructive gates. No delete-one, clear-all, or credential-cleanup action was confirmed or executed.                                                                                                                                                     |
| Uninstall residual scan                                                               | WARN / PARTIAL | NSIS removal with `Delete the application data` selected removed product files and shortcuts; an empty Local `TextbookLens` directory remained. Full Roaming/credential scan was not completed.                                                                                                                               |

### Historical provider defect on the superseded candidate

At `2026-08-09T00:04:33.064Z`, `P15T8-SBX-CRED-03` remained the sole running Sandbox. The user had
entered only DeepSeek and Kimi test credentials directly in the product UI. A user-supplied safe
screenshot plus the controller's read-only live capture showed `DeepSeek · deepseek-v4-flash` and
`Kimi · kimi-k3` as connected, with the registry capability labels rendered. The user confirmed
that OpenAI, Gemini, and Anthropic credentials were unavailable. Task 8 did not read clipboard data
or inspect, extract, display, or move Credential Manager, disk, environment, or Key material. No
Key was requested in chat.

The initial Computer Use input bridge failed with normalized error `node_repl exec context not
found`. An authorized PID/HWND-bound native fallback then verified Sandbox PID `10664`, HWND
`132426`, 144 DPI, and a fresh image before and after each action. It set DeepSeek as the learning
default at `2026-08-09T00:14:54.964Z` and Kimi as the vision default at
`2026-08-09T00:15:24.572Z`; those local routing changes do not count as provider-request PASS.

At `2026-08-09T00:17:57.300Z`, an authoritative foreground capture showed OpenAI selected, a
non-empty masked credential field, and normalized UI error `AI services request failed; check
network or service key`. The user had supplied no OpenAI credential and Task 8 never read the field
or clipboard. An input intended for a safe import control had been derived from a stale
pre-calibration frame and landed on the live Validate-and-connect control; the run stopped
immediately.

`ProviderConnectForm.tsx` changes `kind` and `modelId` on provider selection without clearing
`credential` (lines 65–71), while `submit` pairs the retained secret with the newly selected
provider kind (lines 40–49). The credential value, request body, headers, response body, and remote
identifiers were not inspected or recorded. This is a reproducible cross-provider
credential-retention/routing product FAIL, not a valid OpenAI gate.

| Provider  | Embedded exact model | Text    | Vision / structured                                                          | Region-specific coverage                                   | Status                                                                  |
| --------- | -------------------- | ------- | ---------------------------------------------------------------------------- | ---------------------------------------------------------- | ----------------------------------------------------------------------- |
| OpenAI    | `gpt-5.6`            | NOT RUN | vision NOT RUN; structured page NOT RUN                                      | N/A                                                        | FAIL: unexpected retained-secret validation; not an OpenAI gate         |
| Gemini    | `gemini-3.6-flash`   | NOT RUN | vision NOT RUN; structured page NOT RUN                                      | N/A                                                        | No user test credential; final NOT RUN                                  |
| Anthropic | `claude-sonnet-5`    | NOT RUN | vision NOT RUN; structured page NOT RUN                                      | N/A                                                        | No user test credential; final NOT RUN                                  |
| DeepSeek  | `deepseek-v4-flash`  | NOT RUN | strict-tool NOT RUN; unsupported visual/page local zero-request gate NOT RUN | N/A                                                        | Profile connected; learning-default selection PASS; remote gate stopped |
| Kimi      | `kimi-k3`            | NOT RUN | vision NOT RUN; structured page NOT RUN                                      | Region not surfaced; CN/international remote gates NOT RUN | Profile connected; vision-default selection PASS; remote gates stopped  |

That fail-fast checkpoint ended the superseded-candidate attempt. The defect was subsequently fixed;
the table above remains historical evidence only and is superseded by the current replacement-RC
provider table below. It must not be read as a current OpenAI FAIL or as current DeepSeek/Kimi status.

### Current replacement-RC provider gates

`P15T8-SBX-CRED-03` remained the sole running Sandbox. The user entered only DeepSeek and Kimi test
credentials directly in the product UI. Both profiles remained connected after the replacement-RC
upgrade. The targeted credential-isolation regression passed: changing provider immediately cleared
the password field and disabled Validate, and selecting credential-free OpenAI neither carried a
saved secret nor sent a request. The historical unexpected retained-secret validation above is not
a current OpenAI provider gate.

DeepSeek text passed with a harmless self-made document selection. At decision time
`2026-08-09T03:43:29Z`, `deepseek-v4-flash` had completed the request without a normalized error; no
prompt/response body, header, or remote identifier was retained. DeepSeek structured output and the
independent unsupported-vision local zero-request assertion remain NOT RUN.

Kimi vision passed at `2026-08-09T06:37:49Z` using the user's currently configured credential and
`kimi-k3`: the app's built-in region selection and Explain action targeted only the self-made Tiny
Task 8 PDF, used no typed prompt, reached terminal `completed`, and accurately recognized both safe
fixture labels. Two earlier inability-to-view outcomes used a different Kimi credential and are not
attributed to this RC, model, capture, or request body. A stale historical marker banner was kept
separate from the newly completed request; persistence of a new marker was not independently
confirmed. Kimi text and structured output remain NOT RUN after safe selection attempts could not
reliably initiate a text request. Only the user's available auto-detected region was exercised; the
other Kimi region remains NOT RUN.

| Provider  | Embedded exact model | Text    | Vision / structured                                                          | Region-specific coverage                                    | UTC decision time          | Status                                                                                  |
| --------- | -------------------- | ------- | ---------------------------------------------------------------------------- | ----------------------------------------------------------- | -------------------------- | --------------------------------------------------------------------------------------- |
| OpenAI    | `gpt-5.6`            | NOT RUN | vision NOT RUN; structured page NOT RUN                                      | N/A                                                         | `2026-08-09T07:05:19.978Z` | No test credential; isolation regression PASS with blank/disabled form and zero request |
| Gemini    | `gemini-3.6-flash`   | NOT RUN | vision NOT RUN; structured page NOT RUN                                      | N/A                                                         | `2026-08-09T07:05:19.978Z` | No user test credential; final NOT RUN                                                  |
| Anthropic | `claude-sonnet-5`    | NOT RUN | vision NOT RUN; structured page NOT RUN                                      | N/A                                                         | `2026-08-09T07:05:19.978Z` | No user test credential; final NOT RUN                                                  |
| DeepSeek  | `deepseek-v4-flash`  | PASS    | strict-tool NOT RUN; unsupported visual/page local zero-request gate NOT RUN | N/A                                                         | `2026-08-09T03:43:29Z`     | Harmless self-made text request completed; no normalized error                          |
| Kimi      | `kimi-k3`            | NOT RUN | vision PASS; structured page NOT RUN                                         | Actual auto-detected user region only; other region NOT RUN | `2026-08-09T06:37:49Z`     | Same-credential self-made visual Explain completed accurately                           |

No raw Key, clipboard content, password value, request/response body, header, or remote identifier
was read or recorded. OpenAI/Gemini/Anthropic remain final NOT RUN. `P15T8-SBX-CRED-03` remains
running on the replacement RC at the final evidence checkpoint.

### User-accepted V1 release exceptions

On 2026-08-09, after reviewing the normalized Task 8 evidence, the user explicitly accepted the
following unexecuted or incomplete gates as V1 release exceptions:

- OpenAI, Gemini, and Anthropic real-provider gates because no test credentials were available.
- DeepSeek structured output and the unsupported-vision local zero-request assertion.
- Kimi text, structured output, and the second regional route.
- True Windows system 125%, 150%, and 200% DPI changes; fullscreen/F11; and live-region validation.
- Backup/restore to a second clean profile or session and the post-restore no-Key reconnect check.
- Delete-one, clear-all, and credential-cleanup destructive gates.
- The complete uninstall residual scan; the recorded result remains WARN / PARTIAL.

These exceptions are a bounded V1 release-risk acceptance, not technical PASS results, not permanent
feature guarantees, and not permission to erase or weaken the underlying evidence. All PASS,
PARTIAL, NOT RUN, and WARN rows above remain authoritative. With no unresolved product FAIL on the
replacement RC, Task 8 is **ACCEPTED WITH DOCUMENTED EXCEPTIONS / 用户接受例外后准予进入 Task 9**.
Task 9 remains NOT RUN in this task and must begin separately.

## CI and publication state

`.github/workflows/ci.yml` runs pinned Node/Rust checks for frontend, Rust, generated output,
sensitive files, licenses, fixtures, an A/B/O/P/Q acceptance subset, and a Windows Tauri bundle.
It has no credentials, textbook content, signing key, update endpoint, or publication secret.
Remote CI execution is **NOT RUN** until a pull request or push invokes it.

Code signing, timestamping, updater configuration, update publication, tag creation, push, and
network publication are **NOT CONFIGURED / NOT RUN**. No unsigned artifact may be described as
signed or published.

## Manual gates retained after Task 8

- Clean Windows 11 x64 package entry points: **NSIS PRIMARY PASS; MSI SMOKE PASS**. Full residual
  scan remains **WARN / PARTIAL**.
- Three-format full reading/restart: **PASS**. Backup/restore, book delete/clear, credential cleanup,
  true 125/150/200% DPI, fullscreen, and live-region completion: **NOT RUN or PARTIAL as itemized
  above**.
- Real-provider gates: **DeepSeek text PASS; Kimi vision PASS; OpenAI/Gemini/Anthropic, structured
  output, Kimi text, DeepSeek visual-zero-request, and second Kimi region NOT RUN**.
- The user accepted the itemized incomplete gates as bounded V1 release exceptions. Task 8 release
  decision: **ACCEPTED WITH DOCUMENTED EXCEPTIONS**; this is not an unconditional technical PASS.
- Release signing, timestamping, updater/update publication, clean-device upgrade, and final
  release approval: **NOT CONFIGURED / NOT RUN**.
