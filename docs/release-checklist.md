# Windows release checklist

This checklist is the technical record for a **local, unsigned Windows 11 x64 V1 build**. It is
not a release approval: clean-device installation, uninstall, accessibility hardware checks, and
real-provider gates belong to Tasks 8 and 9 and remain **NOT RUN** until their evidence is added.
Windows 10 is neither supported nor validated.

## Immutable inputs

| Input                  | Required value                                                                                                                                                                                  |
| ---------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Source commit          | `ba0c62dc5e346461bdb1bf2004f4e009f50fbbd3` plus this Task 7 change only                                                                                                                         |
| Node / npm             | `v24.18.1` / `11.16.0`                                                                                                                                                                          |
| Rust / Cargo           | `1.97.1` / `1.97.1`                                                                                                                                                                             |
| Rust target            | `x86_64-pc-windows-msvc`                                                                                                                                                                        |
| Tauri CLI / Rust crate | `2.11.4` / `2.11.5`                                                                                                                                                                             |
| Application version    | `0.1.0` (`package.json`, `Cargo.toml`, and `tauri.conf.json`)                                                                                                                                   |
| Product / identifier   | `TextbookLens` / `dev.textbooklens.desktop`                                                                                                                                                     |
| Dependency records     | `package-lock.json` SHA-256 `21E17A6AEE4AC24DD4401CCEC0ABCD05D32F8CDBEF4838519BE6FBD7149180E2`; `src-tauri/Cargo.lock` SHA-256 `5AFE9A8782AEB56BC9A90F0FD3821DC48249DA8035CFEE818DB527259463E2` |
| Toolchain record       | `rust-toolchain.toml` SHA-256 `6B5C36CC63BE7BF3A075574039B8A49C1361FC1C3ACFCE234AFD28BCC7DECF13`                                                                                                |

No lockfile may change during installation, checking, or bundling. The release build uses
`CARGO_INCREMENTAL=0`, `CARGO_BUILD_JOBS=1`, Cargo `-j 1`, a task-scoped target directory, and a
task-scoped `TEMP`/`TMP` directory.

## Clean-source procedure

Use a Git-byte-preserving local clone/snapshot; do not use a checkout with `core.autocrlf` enabled.
The clone keeps the Git index required by the sensitive-file guard. On this project, CRLF conversion
changes SQLx migration checksums. Before each build, verify every migration hash, including
`0012`–`0015`, against the Git blob bytes.

```powershell
$env:CARGO_TARGET_DIR = 'D:\CodexBuild\textbooklens-p15t7-target-a'
$env:TEMP = 'D:\CodexBuild\textbooklens-p15t7-temp-a'
$env:TMP = $env:TEMP
$env:CARGO_INCREMENTAL = '0'
$env:CARGO_BUILD_JOBS = '1'
git -c core.autocrlf=false clone --no-local . D:\CodexBuild\textbooklens-p15t7-source-a
git -C D:\CodexBuild\textbooklens-p15t7-source-a config core.autocrlf false
Set-Location D:\CodexBuild\textbooklens-p15t7-source-a
npm.cmd ci
npm.cmd ci --offline
cargo.exe metadata --locked --manifest-path src-tauri/Cargo.toml --no-deps
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/preflight.ps1 -Stage Toolchain,Frontend,Rust,Integrity,Acceptance -BuildRoot D:\CodexBuild\textbooklens-p15t7-preflight-a -SkipArtifactBuild -Offline
npm.cmd run tauri build
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
fixtures, credentials, user paths/data, debug symbols, or a development-server URL. The final
artifact directory is `D:\CodexBuild\textbooklens-p15t7-release`; it is retained for Task 8.

| Artifact                           | SHA-256                                                            | Size (bytes) | Scanner / inspection |
| ---------------------------------- | ------------------------------------------------------------------ | -----------: | -------------------- |
| NSIS installer                     | `E77AB16985410A84A04E2965A08DBE0AD974E0646991FA1B5AF6E9D8AFBE38DC` |    9,000,265 | Scanner PASS; x64 PE |
| MSI installer                      | `938AFFB126FE81D9F48493CACFC8EAB9062D086F6DD50E814553796287E3A5DE` |   12,046,336 | Scanner PASS         |
| unpacked bundle/resources manifest | 21 files / 21,108,514 bytes                                        |   21,108,514 | Scanner PASS         |

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

## CI and publication state

`.github/workflows/ci.yml` runs pinned Node/Rust checks for frontend, Rust, generated output,
sensitive files, licenses, fixtures, an A/B/O/P/Q acceptance subset, and a Windows Tauri bundle.
It has no credentials, textbook content, signing key, update endpoint, or publication secret.
Remote CI execution is **NOT RUN** until a pull request or push invokes it.

Code signing, timestamping, updater configuration, update publication, tag creation, push, and
network publication are **NOT CONFIGURED / NOT RUN**. No unsigned artifact may be described as
signed or published.

## Manual gates retained for Task 8/9

- Clean Windows 11 x64 install, launch, uninstall, and residual-data scan: **NOT RUN**.
- First run, three import formats, restart, backup/restore, delete/clear, real DPI/high-contrast,
  and Narrator/NVDA: **NOT RUN**.
- Five real-provider credential/model checks and supported visual/structured requests: **NOT RUN**.
- Release signing, update publication, clean-device upgrade, and final release approval: **NOT RUN**.
