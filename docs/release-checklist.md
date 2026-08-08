# Windows release checklist

This checklist is the technical record for a **local, unsigned Windows 11 x64 V1 build**. It is
not a release approval. Task 8 remains **RED / BLOCKED** after real Windows Sandbox execution:
the NSIS primary flow and MSI independent smoke ran, but required three-format reading,
125/150/200% system scaling, complete accessibility/backup/delete coverage, and all real-provider
requests remain incomplete or NOT RUN. Windows 10 is neither supported nor validated, and Task 9
must not start while these gates remain.

## Immutable inputs

| Input                  | Required value                                                                                                                                                                                    |
| ---------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Source commit          | `bfbe504b0380a64d88c9d51466768195d36b396b` (Task 7 base `ba0c62dc5e346461bdb1bf2004f4e009f50fbbd3`)                                                                                               |
| Node / npm             | `v24.18.1` / `11.16.0`                                                                                                                                                                            |
| Rust / Cargo           | `1.97.1` / `1.97.1`                                                                                                                                                                               |
| Rust target            | `x86_64-pc-windows-msvc`                                                                                                                                                                          |
| Tauri CLI / Rust crate | `2.11.4` / `2.11.5`                                                                                                                                                                               |
| Application version    | `0.1.0` (`package.json`, `Cargo.toml`, and `tauri.conf.json`)                                                                                                                                     |
| Product / identifier   | `TextbookLens` / `dev.textbooklens.desktop`                                                                                                                                                       |
| Dependency records     | `package-lock.json` SHA-256 `E7593397180DCE443149265BF569464823CD43CB5650C586BB05003CD64776B0`; `src-tauri/Cargo.lock` SHA-256 `5AFE9A8782AEB56BC9A90F0FD3821DC48249DA8035CFEE818DB527259463E2EE` |
| Toolchain record       | `rust-toolchain.toml` SHA-256 `6B5C36CC63BE7BF3A075574039B8A49C1361FC1C3ACFCE234AFD28BCC7DECF13`                                                                                                  |

No lockfile may change during installation, checking, or bundling. The release build uses
`CARGO_INCREMENTAL=0`, `CARGO_BUILD_JOBS=1`, Cargo `-j 1`, a task-scoped target directory, and a
task-scoped `TEMP`/`TMP` directory.

## Clean-source procedure

Use a Git-byte-preserving local clone/snapshot; do not use a checkout with `core.autocrlf` enabled.
The clone keeps the Git index required by the sensitive-file guard. On this project, CRLF conversion
changes SQLx migration checksums. Before each build, verify every migration hash, including
`0012`–`0015`, against the Git blob bytes.

```powershell
$env:TEMP = 'D:\CodexBuild\textbooklens-p15t7-temp-e'
$env:TMP = $env:TEMP
$env:CARGO_INCREMENTAL = '0'
$env:CARGO_BUILD_JOBS = '1'
git -c core.autocrlf=false clone --no-local . D:\CodexBuild\textbooklens-p15t7-source-e
git -C D:\CodexBuild\textbooklens-p15t7-source-e checkout --detach bfbe504b0380a64d88c9d51466768195d36b396b
git -C D:\CodexBuild\textbooklens-p15t7-source-e config core.autocrlf false
Set-Location D:\CodexBuild\textbooklens-p15t7-source-e
npm.cmd ci
npm.cmd ci --offline
cargo.exe metadata --locked --manifest-path src-tauri/Cargo.toml --no-deps
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/preflight.ps1 -Stage All -BuildRoot D:\CodexBuild\textbooklens-p15t7-preflight-e -Offline
```

The `npm ci` network-resolution/download phase and `npm ci --offline` cache-only phase must be
recorded separately. A missing cache is **NOT RUN**, never an offline pass. Do not read or write
normal user app data, Documents, or normal Temp during this procedure.

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

| Artifact                           | SHA-256                                                            | Size (bytes) | Scanner / inspection                                                                                       |
| ---------------------------------- | ------------------------------------------------------------------ | -----------: | ---------------------------------------------------------------------------------------------------------- |
| NSIS installer                     | `E77AB16985410A84A04E2965A08DBE0AD974E0646991FA1B5AF6E9D8AFBE38DC` |    9,000,265 | Scanner PASS; Task 8 found an x86 outer bootstrap PE, which is not installed-payload architecture evidence |
| MSI installer                      | `938AFFB126FE81D9F48493CACFC8EAB9062D086F6DD50E814553796287E3A5DE` |   12,046,336 | Scanner PASS                                                                                               |
| unpacked bundle/resources manifest | 21 files / 21,108,514 bytes                                        |   21,108,514 | Scanner PASS                                                                                               |

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

## Task 8 clean-Windows and real-provider result

Task 8 started from exact HEAD `8af9882e44be33e18c91a39e81f324cf98b3b6d2` with parent
`c7716affae03fe820f76aceec34e0055c8105e53` and preserved its initial evidence in
`458ea4f7dc939b070dc58a71290b2a069e601ed2`; the pre-restart continuation was recorded in
`bbe4e2ede0616cd7de210ad5d56feb7d6a4901f5`. The protected Task 7 chain was not rewritten. Start,
environment-switch, and final checks found the retained release directory unchanged at the
size/hash values above. The daily-user desktop shortcut still targets the designated debug
executable, which remains 43,559,424 bytes with SHA-256
`38F6652F6F4B60185046DAB8426A3E9C0B69E912BE75118CA68B19D83F290521`; Task 8 did not launch, stop,
overwrite, or relink it.

### Environment proof and decision

The daily host is Windows 11 Pro 23H2 build `22631.2861`, x64. It remains excluded from clean
installation, destructive, display, theme, and assistive-technology testing. After the authorized
Microsoft Sandbox feature enablement and normal restart, the host reported an active hypervisor and
a runnable Windows Sandbox feature.

Two disposable sessions used the same network-enabled `.wsb` profile with the Task 7 release and
self-made input mapped read-only, a separate writable external-test directory, and clipboard
disabled:

| Evidence ID         | Guest proof                                                    | Initial scale record                                                     | Lifetime / isolation                                                                                                |
| ------------------- | -------------------------------------------------------------- | ------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------- |
| `P15T8-SBX-NSIS-01` | Windows 11 Enterprise 22H2 build `22621.2861`, x64, 4 GiB      | Exact percentage was not surfaced; required 125/150/200% changes NOT RUN | Fresh Sandbox; guest instance `bda89b1d-9684-4cc2-9e17-c4c58c79c059`; destroyed after NSIS flow                     |
| `P15T8-SBX-MSI-02`  | Independent fresh launch of the same Enterprise/x64 base image | Exact percentage was not surfaced; no system-scale claim                 | Fresh Sandbox with no inherited app data or credentials; no reusable snapshot exists; destroyed after MSI uninstall |

Only self-made tiny PDF/EPUB/DOCX/PNG fixtures were exposed to the guest. The primary fixture hashes
were PDF `464847140CCA555A80FF51A0EEBA1072A06175A57239FBF0BE942DAA40B7DF62`, EPUB
`F791BAFA42D089C0081536C53C1B69701DE4019FD6B1EEEECACFA931A5338286`, DOCX
`8017A4B9BEE398496B2F0C8E99084BADF5644960020C57548A18A079E3164595`, and PNG
`6C65EFAA2EBBB9912BA372076E088471EC1F6BF29ED62613C9D30A42569A50EF`. No real textbook, user
database, private account path, or host credential entered either session.

The result remains **RED / BLOCKED** because multiple required manual rows remain NOT RUN. No
product FAIL was observed in the executed subset. Historical automation and browser emulation are
kept separate from the current manual evidence.

### Installer execution and signature state

| Input | Version / architecture evidence                                                                                                                   | Signature state                     | Current clean-Windows execution                                                                                                                                                                                                                        |
| ----- | ------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| NSIS  | File/product version `0.1.0`; outer bootstrap PE `0x014c` (x86); Task 7 payload target remains x86-64. The outer stub alone is not payload proof. | `NotSigned`; no signer or timestamp | Primary networked flow PASS: clean install, first launch, PDF onboarding import, restart, same-version `Upgrade install`, relaunch, and uninstall. A separate offline attempt stopped at the Microsoft WebView2 dependency and is not an offline PASS. |
| MSI   | Product `TextbookLens` `0.1.0`; summary `x64;0`; 64-bit main component; default `C:\Program Files\TextbookLens\`.                                 | `NotSigned`; no signer or timestamp | Independent fresh-session smoke PASS: install, completion launch into first-run onboarding, maintenance-mode Remove, completion, and desktop-shortcut removal. No version-to-version upgrade claim was made.                                           |

Task 7's matching-content but non-bit-reproducible installer-container result remains unchanged.
Signing, timestamping, updater configuration, publication, and remote CI remain **NOT CONFIGURED /
NOT RUN**.

### Manual gate ledger

| Gate                                                                                  | Status         | Current manual evidence                                                                                                                                                                         |
| ------------------------------------------------------------------------------------- | -------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| NSIS primary install/launch/reinstall/uninstall plus MSI independent smoke            | PASS           | Both package entry points ran in separate fresh Sandbox sessions; NSIS same-version upgrade and MSI maintenance removal completed.                                                              |
| First run and self-made PDF import/restart/source deletion                            | PARTIAL        | PDF title/import state survived restart and deletion of only the scoped writable source duplicate. API-key onboarding prevented full reader-content verification.                               |
| Self-made EPUB/DOCX import and full three-format reader restart                       | NOT RUN        | Onboarding could not proceed beyond the required credential step; no result was inferred from historical automation.                                                                            |
| `zh-CN` / `zh-TW` / `en` switching and restart persistence                            | PASS           | All three UI languages were selected in the installed NSIS candidate; English persisted after restart.                                                                                          |
| True 125/150/200% system scaling and VM resolution/display resize                     | NOT RUN        | Sandbox Settings did not expose a usable display-scale path; browser zoom/emulation was not substituted.                                                                                        |
| Maximized and narrow readability                                                      | PASS           | Installed onboarding surface remained readable maximized and in an approximately 810-pixel snapped width.                                                                                       |
| Fullscreen/F11                                                                        | NOT RUN        | No fullscreen PASS is claimed.                                                                                                                                                                  |
| Windows Contrast Theme, transparency off, opaque fallback, reduced motion             | PASS (surface) | Aquatic Contrast Theme, transparency off, and animation effects off remained functional and readable on the installed onboarding surface; settings were restored/disposed.                      |
| Keyboard-only skip/focus/provider/model/Esc-return path                               | PASS (surface) | Skip link, provider selection, advanced model disclosure, and Esc focus return worked. Complete live-region coverage was not established.                                                       |
| Built-in Narrator                                                                     | PASS (surface) | Narrator was enabled through Settings and navigated the key and advanced-model controls; it was then disabled.                                                                                  |
| NVDA                                                                                  | NOT RUN        | NVDA was not already installed and no separate official download/install run was completed.                                                                                                     |
| Backup to external test directory and restore to a second clean profile/session       | NOT RUN        | No backup/restore or no-Key reconnect evidence was produced.                                                                                                                                    |
| Delete one book, clear all, second-book/original/backup retention, credential cleanup | NOT RUN        | The scoped source-copy deletion is not a product book-delete or clear-all test; no credentials were configured.                                                                                 |
| Uninstall residual scan                                                               | WARN / PARTIAL | NSIS removal with `Delete the application data` selected removed product files and shortcuts; an empty Local `TextbookLens` directory remained. Full Roaming/credential scan was not completed. |

### Real-provider manual gates

At `2026-08-08T12:22:31.658Z`, credential availability had not been established in an isolated
release-candidate UI. The clean Sandbox sessions inherited no host credentials. The daily-host
candidate was not launched for discovery because process-only `APPDATA`/`LOCALAPPDATA` overrides do
not isolate its `FOLDERID_RoamingAppData` storage, and Task 8 did not inspect, extract, display, or
move Credential Manager, disk, or environment secrets. No Key was read or requested in chat, and
zero external-provider requests were made.

| Provider  | Embedded exact model | Text    | Vision / structured                                                          | Region-specific coverage                                   | Status                                                                     |
| --------- | -------------------- | ------- | ---------------------------------------------------------------------------- | ---------------------------------------------------------- | -------------------------------------------------------------------------- |
| OpenAI    | `gpt-5.6`            | NOT RUN | vision NOT RUN; structured page NOT RUN                                      | N/A                                                        | Credential not configured in disposable UI; availability otherwise unknown |
| Gemini    | `gemini-3.6-flash`   | NOT RUN | vision NOT RUN; structured page NOT RUN                                      | N/A                                                        | Credential not configured in disposable UI; availability otherwise unknown |
| Anthropic | `claude-sonnet-5`    | NOT RUN | vision NOT RUN; structured page NOT RUN                                      | N/A                                                        | Credential not configured in disposable UI; availability otherwise unknown |
| DeepSeek  | `deepseek-v4-flash`  | NOT RUN | strict-tool NOT RUN; unsupported visual/page local zero-request gate NOT RUN | N/A                                                        | Credential not configured in disposable UI; availability otherwise unknown |
| Kimi      | `kimi-k3`            | NOT RUN | vision NOT RUN; structured page NOT RUN                                      | CN and international probe/chat/Files/cleanup both NOT RUN | Neither regional test credential was configured in disposable UI           |

Any resumed provider gate requires the user to enter the relevant test Key manually in a
disposable TextbookLens UI. Keys must not be pasted into chat or copied from host storage.

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
- Three-format full reading, backup/restore, book delete/clear, credential cleanup, true
  125/150/200% DPI, resolution/fullscreen, live-region completion, and NVDA: **BLOCKED / NOT RUN or
  PARTIAL as itemized above**.
- Five real-provider credential/model checks and supported visual/structured requests:
  **BLOCKED / NOT RUN**.
- Release signing, timestamping, updater/update publication, clean-device upgrade, and final
  release approval: **NOT CONFIGURED / NOT RUN**.
