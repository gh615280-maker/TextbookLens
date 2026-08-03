# TextbookLens 重规划验证矩阵

本文件只记录 normalized pass/fail evidence。禁止记录 credential、教材正文、页面图像、完整 prompt/context、教学指令、provider response body 或回答。

## 1. 基线

| 范围                  | 基线 HEAD/证据                        | 状态                       | 说明                                                    |
| --------------------- | ------------------------------------- | -------------------------- | ------------------------------------------------------- |
| Phase 1               | `docs/testing/verification-matrix.md` | INHERITED PASS             | skeleton/security/credential/CI                         |
| Phase 2               | `docs/testing/verification-matrix.md` | INHERITED PASS             | PDF/EPUB/DOCX import/local index/restart/source removal |
| Phase 3               | `docs/testing/verification-matrix.md` | INHERITED PASS             | three readers/text anchors/markers/restart              |
| Old Phase 4 Tasks 1–7 | commits `51f3f1b`…`9834e25`           | IMPLEMENTED; PHASE 4R PASS | five text adapters plus shared proof at `e4d12c2`       |
| Replanning Stage 0    | `2026-08-03` docs                     | IN PROGRESS                | no product implementation                               |

## 2. 阶段 checkpoint

| 阶段     | 必需证据                                           | 状态    | Date/HEAD                                                       | Evidence summary                                                                                                                                                                                                                                                                                            |
| -------- | -------------------------------------------------- | ------- | --------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Phase 4R | five-provider text/terminal/cancel/error/redaction | PASS    | 2026-08-03 / `e4d12c2`                                          | 8 shared contract tests; 25 provider tests; 7 stream tests; full Rust gate 117 passed, 1 pre-existing manual keyring smoke ignored; no real credentials                                                                                                                                                     |
| Phase 5  | capability/vision/structured/unknown gate          | PASS    | 2026-08-03 / `b36892ec2280af031a9ce9ce667596c9bb11cb41`         | 8 operation-contract tests; text retained for all five providers; exact supported/denied capability routes; 32 invalid structured-result cases; deterministic cancellation at all required boundaries; redaction, fixtures, sensitive, generated, typecheck, Rust, dependency, and Tauri debug gates passed |
| Phase 6  | three-language/Fluent/reader shell                 | BLOCKED | 2026-08-03 / product `7bff4d8353a3ba23752adfa932245765e652f8ed` | Automated checkpoint gates PASS (124 Vitest, 9 Playwright, generated, Rust settings, frontend/Tauri builds). Required durable Windows 10 and 200% manual screenshot evidence is unavailable; the inherited EPUB adapter still requests paginated flow, so the continuous-scroll contract is not proven.     |
| Phase 7  | atomic credential/book→Key→read/AI services        | NOT RUN | —                                                               | —                                                                                                                                                                                                                                                                                                           |
| Phase 8  | teaching instruction/revision/test no-history      | NOT RUN | —                                                               | —                                                                                                                                                                                                                                                                                                           |
| Phase 9  | page index/resume/partial/correction/provenance    | NOT RUN | —                                                               | —                                                                                                                                                                                                                                                                                                           |
| Phase 10 | Explorer library/drop/import/index status          | NOT RUN | —                                                               | —                                                                                                                                                                                                                                                                                                           |
| Phase 11 | text/region anchors/preparation/notes              | NOT RUN | —                                                               | —                                                                                                                                                                                                                                                                                                           |
| Phase 12 | registry/atomic history/multi-panel/hide continue  | NOT RUN | —                                                               | —                                                                                                                                                                                                                                                                                                           |
| Phase 13 | delete/backup/restore/clear/privacy                | NOT RUN | —                                                               | —                                                                                                                                                                                                                                                                                                           |
| Phase 14 | deterministic overview/book questions              | NOT RUN | —                                                               | —                                                                                                                                                                                                                                                                                                           |
| Phase 15 | A–Q/clean Windows/package/release                  | NOT RUN | —                                                               | —                                                                                                                                                                                                                                                                                                           |

## 3. Provider 能力证据

不在规划阶段预填不稳定事实。Phase 5 按当日官方来源填写；unknown 不等于 fail，也不允许 UI 使用。

| Provider/model                 | text                                    | image          | PDF/page operation                     | strict structured                                   | limits verified                                                                                | official URL/date                                                                                                                                                                                                                                                | contract result                                            |
| ------------------------------ | --------------------------------------- | -------------- | -------------------------------------- | --------------------------------------------------- | ---------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------- |
| OpenAI / `gpt-5.6`             | SUPPORTED; regression PASS              | SUPPORTED      | PDF SUPPORTED; page operation PASS     | SUPPORTED                                           | 1,050,000 input; 128,000 output; app image limits 4 / 4 MiB / 12 MiB / 4,096 px / 8,847,360 px | [model](https://developers.openai.com/api/docs/models/gpt-5.6-sol), [vision](https://developers.openai.com/api/docs/guides/images-vision), [structured](https://developers.openai.com/api/docs/guides/structured-outputs) / 2026-08-03                           | PASS: text, vision, structured                             |
| Gemini / `gemini-3.6-flash`    | SUPPORTED; regression PASS              | SUPPORTED      | PDF SUPPORTED; page operation PASS     | SUPPORTED                                           | 1,048,576 input; 65,536 output; app image limits 4 / 4 MiB / 12 MiB / 4,096 px / 8,847,360 px  | [model](https://ai.google.dev/gemini-api/docs/models/gemini-3.6-flash), [vision](https://ai.google.dev/gemini-api/docs/image-understanding), [structured](https://ai.google.dev/gemini-api/docs/generate-content/structured-output) / 2026-08-03                 | PASS: text, vision, structured                             |
| Anthropic / `claude-sonnet-5`  | SUPPORTED; regression PASS              | SUPPORTED      | PDF SUPPORTED; page operation PASS     | SUPPORTED                                           | 1,000,000 input; 128,000 output; app image limits 4 / 4 MiB / 12 MiB / 4,096 px / 8,847,360 px | [model](https://platform.claude.com/docs/en/about-claude/models/whats-new-sonnet-5), [vision](https://platform.claude.com/docs/en/build-with-claude/vision), [structured](https://platform.claude.com/docs/en/build-with-claude/structured-outputs) / 2026-08-03 | PASS: text, vision, structured                             |
| DeepSeek / `deepseek-v4-flash` | SUPPORTED; regression PASS              | UNSUPPORTED    | PDF UNSUPPORTED; page operation DENIED | SUPPORTED for strict tools; visual aggregate DENIED | 1,000,000 input; 393,216 output; no visual limits                                              | [model facts](https://api-docs.deepseek.com/quick_start/pricing/), [chat schema](https://api-docs.deepseek.com/api/create-chat-completion/), [strict tools](https://api-docs.deepseek.com/guides/tool_calls/) / 2026-08-03                                       | PASS: text; visual routes denied before credential/network |
| Kimi / `kimi-k3`               | SUPPORTED; regression PASS              | SUPPORTED      | PDF UNSUPPORTED; page operation PASS   | SUPPORTED                                           | 1,048,576 input/output; app image limits 4 / 4 MiB / 12 MiB / 4,096 px / 8,847,360 px          | [model](https://platform.kimi.ai/docs/models), [K3](https://platform.kimi.ai/docs/guide/kimi-k3-quickstart), [vision](https://platform.kimi.ai/docs/guide/use-kimi-vision-model) / 2026-08-03                                                                    | PASS: text, vision, structured                             |
| Unknown/custom model rule      | conservative only after text validation | UNKNOWN/DENIED | UNKNOWN/DENIED                         | UNKNOWN/DENIED                                      | 32,000-token text fallback only                                                                | ADR 0005 / 2026-08-03                                                                                                                                                                                                                                            | PASS: vision/structured denied before credential/network   |

## 4. A–Q 验收

| ID  | 场景                            | 自动化 owner     | 必需手工补充                       | 状态    | Evidence                                                                                                                                                                                                 |
| --- | ------------------------------- | ---------------- | ---------------------------------- | ------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| A   | book→Key→read                   | Phase 7/15       | clean keyring onboarding           | NOT RUN | —                                                                                                                                                                                                        |
| B   | zh-CN/zh-TW/en                  | Phase 6/15       | Windows locale/scale               | BLOCKED | Three-language immediate switch and restart mock PASS in Playwright; actual Windows 10/200% manual evidence unavailable.                                                                                 |
| C   | teaching instruction            | Phase 8/15       | copy/usability review              | NOT RUN | —                                                                                                                                                                                                        |
| D   | direct text explanation         | Phase 11–12/15   | real reader selection              | NOT RUN | —                                                                                                                                                                                                        |
| E   | region containing reliable text | Phase 11/15      | three-format visual check          | NOT RUN | —                                                                                                                                                                                                        |
| F   | chart/image region confirmation | Phase 11–12/15   | verified vision provider           | NOT RUN | —                                                                                                                                                                                                        |
| G   | multiple floating panels        | Phase 12/15      | drag/resize/scale                  | NOT RUN | —                                                                                                                                                                                                        |
| H   | hidden panel continues          | Phase 12/15      | real window interaction            | NOT RUN | —                                                                                                                                                                                                        |
| I   | history continue/delete         | Phase 12/15      | restart UX                         | NOT RUN | —                                                                                                                                                                                                        |
| J   | decline scanned-PDF AI          | Phase 9/15       | network observation                | NOT RUN | —                                                                                                                                                                                                        |
| K   | AI-assisted local index         | Phase 9/15       | verified provider + offline search | NOT RUN | —                                                                                                                                                                                                        |
| L   | partial failure/retry           | Phase 9/15       | restart/recovery                   | NOT RUN | —                                                                                                                                                                                                        |
| M   | manual text/LaTeX correction    | Phase 9/15       | side-by-side usability             | NOT RUN | —                                                                                                                                                                                                        |
| N   | provenance distinction          | Phase 9/11/14/15 | citation copy review               | NOT RUN | —                                                                                                                                                                                                        |
| O   | responsive/fullscreen           | Phase 6/12/15    | Win10/11, 200%, high contrast      | BLOCKED | Narrow/forced-colors/axe and F11/Esc automation PASS; actual Windows 11 build 22631 debug window inspected with a synthetic fixture, but no durable Windows 10 or 200% screenshot artifact was produced. |
| P   | backup/restore no Key           | Phase 13/15      | second clean profile               | NOT RUN | —                                                                                                                                                                                                        |
| Q   | delete/clear/leak               | Phase 13/15      | credential/app-data residual scan  | NOT RUN | —                                                                                                                                                                                                        |

## 5. Race/restart matrix

| Boundary        | Required cases                                               | Owner                | Status                                                                                                                                  |
| --------------- | ------------------------------------------------------------ | -------------------- | --------------------------------------------------------------------------------------------------------------------------------------- |
| import          | copy/parse/index cancel and restart                          | inherited + Phase 15 | PENDING RECHECK                                                                                                                         |
| provider stream | cancel before headers/delta/terminal, EOF/malformed          | Phase 4R/5           | PHASE 4R PASS; PHASE 5 PASS: pre-encode, between-frame, and post-terminal/pre-return cancellation; corrupt/malformed/EOF never complete |
| page index      | claim owner, late response, validate/commit, partial restart | Phase 9              | NOT RUN                                                                                                                                 |
| learning        | hide/unmount, cancel vs terminal vs DB commit, two requests  | Phase 12             | NOT RUN                                                                                                                                 |
| deletion        | trash before/after DB commit, remote cleanup fail            | Phase 13             | NOT RUN                                                                                                                                 |
| backup          | active work, output temp/rename failure                      | Phase 13             | NOT RUN                                                                                                                                 |
| restore         | corrupt archive, every swap fault, rollback/reopen           | Phase 13             | NOT RUN                                                                                                                                 |
| clear all       | key deletion/data swap/restart                               | Phase 13             | NOT RUN                                                                                                                                 |

## 6. Privacy artifacts

| Artifact                       | Forbidden sentinel scan                                              | Owner          | Status                                                                                                                          |
| ------------------------------ | -------------------------------------------------------------------- | -------------- | ------------------------------------------------------------------------------------------------------------------------------- |
| logs / error DTO / diagnostics | Key, path, content, images, prompt, answer, instruction, vendor body | every phase/15 | PHASE 4R PASS; PHASE 5 PASS: raw image/schema/vendor/credential/hidden sentinels absent from errors, DTOs, sources, and tracing |
| SQLite                         | Key/raw page image/provider body/incomplete stream                   | Phase 9/12/13  | NOT RUN                                                                                                                         |
| `.tlbackup`                    | all credentials/IDs, logs, scratch, remote handle, incomplete output | Phase 13/15    | NOT RUN                                                                                                                         |
| frontend bundle/source map     | credential/content fixtures, custom provider URL                     | Phase 15       | NOT RUN                                                                                                                         |
| Tauri resources/installer      | secrets/user data/unsafe fixtures                                    | Phase 15       | NOT RUN                                                                                                                         |

## 7. Global release commands

Record exact date, HEAD, exit result, test count and only non-sensitive warnings when executed:

| Command group                                   | Status                 | Evidence                                                                                                                                                      |
| ----------------------------------------------- | ---------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `npm ci`, Chromium install                      | NOT RUN                | —                                                                                                                                                             |
| format/lint/typecheck/test/build/e2e            | PHASE 6 AUTO PASS      | 2026-08-03 product `7bff4d8`; format/lint/typecheck/build PASS; 124 Vitest and 9 Playwright passed. Required Phase 6 manual evidence remains BLOCKED.         |
| generated/sensitive/licenses/fixtures           | PHASE 6 GENERATED PASS | generated binding export 1 passed; Phase 5 sensitive/fixtures/licenses evidence remains unchanged; npm release-license gate remains NOT RUN.                  |
| cargo fmt/clippy/test all-features `-j 1`       | PHASE 5 PASS           | `b36892e`; fmt and clippy PASS; 155 passed, 0 failed, 1 ignored pre-existing manual credential smoke                                                          |
| cargo deny                                      | PHASE 5 PASS           | `b36892e`; advisories, bans, licenses, and sources PASS                                                                                                       |
| Tauri debug/no-bundle and release bundle        | PARTIAL                | Phase 6 debug/no-bundle PASS at product `7bff4d8` (1,869 frontend modules); release bundle remains NOT RUN.                                                   |
| clean Windows 10/11 install                     | BLOCKED                | Windows 11 Pro build 22631 debug executable launched and inspected with synthetic content; clean Windows 10 and durable 200% screenshot evidence unavailable. |
| five-provider text + verified capability manual | NOT RUN                | —                                                                                                                                                             |

## 8. Evidence update rules

- `PASS` 只在命令/场景实际完成且无必需工作剩余时填写。
- 环境/服务故障写 `BLOCKED` 和 safe code，不写 provider body。
- 一项 provider 能力官方未知写 `UNKNOWN/DENIED`，不是伪造 PASS/FAIL。
- 每次 checkpoint 记录最终 HEAD；测试中使用自制内容，只记录计数/状态。

## 9. Phase 4R normalized evidence

All commands below completed on 2026-08-03 against code HEAD `e4d12c2`. Tests used the checked-in
synthetic provider fixtures, loopback HTTP servers, and synthetic credential strings; no real
credential or provider network call was used.

| Command                                                                                         | Result | Normalized evidence                                                                                                                                                                                 |
| ----------------------------------------------------------------------------------------------- | ------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `cargo test --manifest-path src-tauri/Cargo.toml provider_contract -j 1`                        | PASS   | 8 passed; five validation routes; provider-specific visible/hidden/terminal vectors; 37 applicable no-completion cases; 15 deterministic cancellation scenarios (five providers × three boundaries) |
| `cargo test --manifest-path src-tauri/Cargo.toml ai::providers -j 1`                            | PASS   | 25 passed                                                                                                                                                                                           |
| `cargo test --manifest-path src-tauri/Cargo.toml ai::stream -j 1`                               | PASS   | 7 passed                                                                                                                                                                                            |
| `cargo fmt --manifest-path src-tauri/Cargo.toml -- --check`                                     | PASS   | no formatting diff                                                                                                                                                                                  |
| `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- -D warnings` | PASS   | no warning or error                                                                                                                                                                                 |
| `cargo test --manifest-path src-tauri/Cargo.toml --all-features -j 1`                           | PASS   | 117 passed; 0 failed; 1 ignored manual disposable Windows Credential Manager smoke test                                                                                                             |
| `npm.cmd run check:generated`                                                                   | PASS   | binding export test: 1 passed                                                                                                                                                                       |
| `npm.cmd run check:sensitive`                                                                   | PASS   | sensitive-file guard passed                                                                                                                                                                         |

The shared contract also probes normalized 401/credential, model-missing, permission, rate/quota,
context, refusal, ambiguous Gemini 429, 5xx, and offline outcomes. Unique sentinels cover the Key,
system/user/assistant text, visible delta, nested vendor body, and Unicode error; none entered
`AiError` debug/display/source, the serialized `AppErrorDto`, or captured tracing. The Windows
linker printed localized import-library creation notices during tests; these were non-sensitive
warnings and did not affect results. PowerShell blocked the `npm.ps1` shim, so the equivalent
Windows executable `npm.cmd` ran both required npm gates successfully.

## 10. Phase 5 normalized evidence

All commands below completed on 2026-08-03 against implementation HEAD
`b36892ec2280af031a9ce9ce667596c9bb11cb41`. The tests used loopback servers, synthetic
credentials, and project-owned 2 × 2 PNG/JPEG fixtures only. They made no real provider call and
contained no textbook content.

| Command                                                                                         | Result | Normalized evidence                                                                                                                                                                                                                                      |
| ----------------------------------------------------------------------------------------------- | ------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `cargo test --manifest-path src-tauri/Cargo.toml operation_contract -j 1`                       | PASS   | 8 passed, 0 failed, 96 filtered; all five text routes; four supported visual/structured routes; five-provider unknown-model plus exact DeepSeek denial before credential/network; 32 structured failure vectors; 16 deterministic cancellation scenarios |
| `cargo test --manifest-path src-tauri/Cargo.toml --test redaction`                              | PASS   | 4 passed, 0 failed                                                                                                                                                                                                                                       |
| `npm.cmd run fixtures:verify`                                                                   | PASS   | fixture integrity passed, including the two project-owned 2 × 2 vision images                                                                                                                                                                            |
| `npm.cmd run check:sensitive`                                                                   | PASS   | sensitive-file guard passed                                                                                                                                                                                                                              |
| `npm.cmd run check:generated`                                                                   | PASS   | binding export test: 1 passed, 0 failed, 4 filtered                                                                                                                                                                                                      |
| `npm.cmd run typecheck`                                                                         | PASS   | TypeScript check completed without error                                                                                                                                                                                                                 |
| `cargo fmt --manifest-path src-tauri/Cargo.toml -- --check`                                     | PASS   | no formatting diff                                                                                                                                                                                                                                       |
| `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- -D warnings` | PASS   | no warning or error                                                                                                                                                                                                                                      |
| `cargo test --manifest-path src-tauri/Cargo.toml --all-features -j 1`                           | PASS   | 155 passed, 0 failed, 1 ignored pre-existing manual disposable Windows Credential Manager smoke test                                                                                                                                                     |
| `cargo deny --manifest-path src-tauri/Cargo.toml check`                                         | PASS   | advisories, bans, licenses, and sources passed                                                                                                                                                                                                           |
| `npm.cmd run tauri build -- --debug --no-bundle`                                                | PASS   | production frontend built from 1,855 transformed modules; debug executable produced without bundling                                                                                                                                                     |

The operation contract proves that corrupt outer JSON, wrong schema version, oversized visible
output, missing pages, duplicate pages, hidden-only output, refusal, and EOF never return a page
batch for any supported visual provider. Cancellation wins before encoding for both visual
operations, between stream frames, and after terminal parsing but before completion is returned.
All implemented routes use inline images: success, failure, and cancellation produced zero upload
or cleanup requests, satisfying the cleanup-at-most-once invariant. Raw image, schema, vendor,
credential, and hidden-content sentinels were absent from `AiError` debug/display/source,
serialized `AppErrorDto`, and captured tracing.

The production boundary audit found only the five allowlisted HTTPS provider origins in
`src-tauri/src/ai/transport.rs`; the frontend contains no provider fetch client or provider URL,
and the CSP does not grant external provider connections. Provider HTTP uses `reqwest 0.13.4`
with Rustls, JSON, stream, gzip, and Brotli features, with redirect, referer, and proxy use disabled.
No reqwest cookie store, native TLS, Hyper-TLS, or OpenSSL dependency is enabled; the separate
`cookie` crate path belongs only to the Tauri webview runtime. Production AI modules contain no
direct tracing or print calls; normalized application logging remains behind the redacting sink.

Normalized non-blocking warnings were the localized Windows linker import-library notices and
Vite's production chunk-size warning (largest emitted assets were the PDF worker at 2,222.99 kB
and JavaScript at 2,118.89 kB, gzip 622.66 kB). Neither warning exposed sensitive content or
invalidated a required gate. ADR 0004/0005 and registry truth required no change.

## 11. Phase 6 normalized evidence

Automated commands completed on 2026-08-03 against product HEAD
`7bff4d8353a3ba23752adfa932245765e652f8ed` plus the Task 6 e2e checkpoint files. Tests used
only checked-in synthetic fixtures and mock IPC; no credential, provider request, or real textbook
content was used for automated evidence.

| Command                                                         | Result | Normalized evidence                                                                                                                                                |
| --------------------------------------------------------------- | ------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `npm.cmd run format:check`                                      | PASS   | all checked files matched Prettier                                                                                                                                 |
| `npm.cmd run lint`                                              | PASS   | zero warnings/errors under `--max-warnings 0`                                                                                                                      |
| `npm.cmd run typecheck`                                         | PASS   | TypeScript project build completed without error                                                                                                                   |
| `npm.cmd test`                                                  | PASS   | 35 files; 124 passed, 0 failed                                                                                                                                     |
| `npm.cmd run build`                                             | PASS   | 1,869 modules transformed; production bundle completed                                                                                                             |
| `npm.cmd run test:e2e`                                          | PASS   | Chromium: 9 passed, 0 failed; three languages/restart mock, normal/onboarding/reader shells, skip link, toolbar lifecycle, F11/Esc, narrow, forced colors, and axe |
| `npm.cmd run check:generated`                                   | PASS   | binding export: 1 passed, 0 failed, 5 filtered                                                                                                                     |
| `cargo test --manifest-path src-tauri/Cargo.toml settings -j 1` | PASS   | 3 matching tests passed across unit/database contract targets; 0 failed                                                                                            |
| `npm.cmd run tauri build -- --debug --no-bundle`                | PASS   | frontend rebuilt from 1,869 modules; debug executable produced without bundling                                                                                    |

The Task 5 focused gate passed 11 tests across `ReaderLayout`, `ReaderPage`, and
`ReaderController`. The minimal toolbar contains only library, contents, book/location, search,
disabled area-selection placeholder, reading settings, and fullscreen. Drawer/settings/language
state does not reopen the adapter in the covered lifecycle tests. First-reader hint completion is
persisted through the existing command and no Phase 11 selection UI was enabled.

Required manual checkpoint status is **BLOCKED**, not PASS. The debug executable was launched on
Windows 11 Pro build 22631 and the opaque Fluent fallback plus reader toolbar were inspected using
a synthetic local fixture. Mica is not enabled in the current Tauri configuration. A clean Windows
10 environment and durable 200% scaling screenshot artifact were unavailable. Two attempts to
exercise WebView zoom did not produce a verifiable scale change and were stopped; browser forced-
colors/narrow automation is not substituted for the missing manual artifact.

An inherited contract defect also remains outside this packet's allowed files:
`EpubReaderAdapter` requests `flow: 'paginated'`, while the replanned specification requires
default continuous scrolling. Phase 6 therefore remains BLOCKED until an authorized adapter owner
changes and verifies that behavior and the required Windows manual evidence is captured.

Normalized non-blocking warnings were Vite's chunk-size warning (PDF worker 2,222.99 kB; main
JavaScript 2,096.41 kB, gzip 615.15 kB), localized Windows linker import-library messages, and Git
line-ending notices. No warning exposed sensitive content.
