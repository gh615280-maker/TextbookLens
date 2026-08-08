# Windows release checklist

This checklist is the technical record for a **local, unsigned Windows 11 x64 V1 build**. It is
not a release approval. Task 8 performed read-only package and environment discovery but was
**RED / BLOCKED** because no genuinely isolated clean Windows environment was available; clean
installation, uninstall, accessibility hardware checks, and real-provider gates remain **NOT RUN**.
Windows 10 is neither supported nor validated, and Task 9 must not start while these gates remain.

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
`c7716affae03fe820f76aceec34e0055c8105e53`. Start and final checks found the retained release
directory unchanged: it contained only the authoritative NSIS and MSI files at the size/hash values
above. The desktop shortcut and its designated debug executable also retained the protected target,
43,559,424-byte size, and SHA-256
`38F6652F6F4B60185046DAB8426A3E9C0B69E912BE75118CA68B19D83F290521`.

### Environment proof and decision

The daily host reported Windows 11 Pro 23H2 build `22631.2861`, x64, but it was not a clean test
environment and was never used as one. Windows Sandbox was not present, no active hypervisor or
Hyper-V management command was available, VMware/VirtualBox management tools were absent, and no
VM/Sandbox configuration, checkpoint, or dedicated test-machine configuration was found in the
repository or task build root. Optional-feature state required elevation and was left unchanged.
No environment switch occurred, so no snapshot identifier exists.

The result is **RED / BLOCKED**. Task 8 did not install/uninstall either package, launch or stop the
existing app, alter current-user app data or credentials, change system display/accessibility
settings, or run a provider request. Historical automation and browser emulation are not clean
Windows evidence.

### Read-only installer inspection

| Input | Metadata result                                                                                                                                                                                                  | Signature state                     | Executed gate                                          |
| ----- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------- | ------------------------------------------------------ |
| NSIS  | `TextbookLens` file/product version `0.1.0`; outer bootstrap machine `0x014c` (x86). The x64 payload claim was not inferred from the stub and requires installed-candidate evidence.                             | `NotSigned`; no signer or timestamp | Install/launch/uninstall **NOT RUN**                   |
| MSI   | Product/version `TextbookLens` / `0.1.0`; summary template `x64;0`; main EXE component is marked 64-bit with file version `0.1.0.0`; Start Menu, desktop, uninstall, and `RemoveExistingProducts` entries exist. | `NotSigned`; no signer or timestamp | Install/launch/reinstall/upgrade/uninstall **NOT RUN** |

MSI database rows are only package metadata. They do not prove a successful entry point, upgrade,
launch, or removal on clean Windows.

### Manual gate ledger

| Gate                                                                                            | Status                                 |
| ----------------------------------------------------------------------------------------------- | -------------------------------------- |
| NSIS and MSI independent installation/start/uninstall; first run; reinstall/supported upgrade   | NOT RUN — no clean Windows environment |
| Self-made PDF/EPUB/DOCX import/read/restart/source deletion                                     | NOT RUN                                |
| `zh-CN` / `zh-TW` / `en` switch and restart persistence                                         | NOT RUN                                |
| Real 125/150/200% scale, display resize, narrow/maximized/fullscreen, Esc/focus                 | NOT RUN                                |
| Contrast Theme, transparency off, opaque fallback, reduced motion                               | NOT RUN                                |
| Keyboard-only, skip/focus/live-region, Narrator, NVDA                                           | NOT RUN                                |
| Backup to external test directory and restore into a second clean environment; no-Key reconnect | NOT RUN                                |
| Delete one book, clear all/credential cleanup, uninstall residual scan                          | NOT RUN                                |

At `2026-08-08T10:21:15.925Z`, the embedded exact models were OpenAI `gpt-5.6`, Gemini
`gemini-3.6-flash`, Anthropic `claude-sonnet-5`, DeepSeek `deepseek-v4-flash`, and Kimi `kimi-k3`.
Text was **NOT RUN** for all five. Supported vision/structured operations were **NOT RUN**;
DeepSeek's unsupported visual/page zero-request gate was also **NOT RUN**. Kimi CN and international
probe/chat/Files/cleanup were both **NOT RUN**. No credential source was inspected, no Key was read
or copied, and Task 8 made zero external-provider requests.

Resume requires a genuine clean Windows 11 x64 VM/Sandbox/dedicated machine with a disposable
snapshot. Dedicated provider test keys must be entered manually in the installed candidate UI, not
provided in chat.

## CI and publication state

`.github/workflows/ci.yml` runs pinned Node/Rust checks for frontend, Rust, generated output,
sensitive files, licenses, fixtures, an A/B/O/P/Q acceptance subset, and a Windows Tauri bundle.
It has no credentials, textbook content, signing key, update endpoint, or publication secret.
Remote CI execution is **NOT RUN** until a pull request or push invokes it.

Code signing, timestamping, updater configuration, update publication, tag creation, push, and
network publication are **NOT CONFIGURED / NOT RUN**. No unsigned artifact may be described as
signed or published.

## Manual gates retained after Task 8

- Clean Windows 11 x64 install, launch, uninstall, and residual-data scan: **BLOCKED / NOT RUN**.
- First run, three import formats, restart, backup/restore, delete/clear, real DPI/high-contrast,
  and Narrator/NVDA: **BLOCKED / NOT RUN**.
- Five real-provider credential/model checks and supported visual/structured requests:
  **BLOCKED / NOT RUN**.
- Release signing, timestamping, updater/update publication, clean-device upgrade, and final
  release approval: **NOT CONFIGURED / NOT RUN**.
