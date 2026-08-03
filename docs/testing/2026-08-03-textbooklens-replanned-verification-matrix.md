# TextbookLens 重规划验证矩阵

本文件只记录 normalized pass/fail evidence。禁止记录 credential、教材正文、页面图像、完整 prompt/context、教学指令、provider response body 或回答。

## 1. 基线

| 范围                  | 基线 HEAD/证据                        | 状态                            | 说明                                                    |
| --------------------- | ------------------------------------- | ------------------------------- | ------------------------------------------------------- |
| Phase 1               | `docs/testing/verification-matrix.md` | INHERITED PASS                  | skeleton/security/credential/CI                         |
| Phase 2               | `docs/testing/verification-matrix.md` | INHERITED PASS                  | PDF/EPUB/DOCX import/local index/restart/source removal |
| Phase 3               | `docs/testing/verification-matrix.md` | INHERITED PASS                  | three readers/text anchors/markers/restart              |
| Old Phase 4 Tasks 1–7 | commits `51f3f1b`…`9834e25`           | IMPLEMENTED; PHASE 4R PASS      | five text adapters plus shared proof at `e4d12c2`       |
| Replanning Stage 0    | `2026-08-03` docs                     | IN PROGRESS                     | no product implementation                               |

## 2. 阶段 checkpoint

| 阶段     | 必需证据                                           | 状态    | Date/HEAD | Evidence summary |
| -------- | -------------------------------------------------- | ------- | --------- | ---------------- |
| Phase 4R | five-provider text/terminal/cancel/error/redaction | PASS    | 2026-08-03 / `e4d12c2` | 8 shared contract tests; 25 provider tests; 7 stream tests; full Rust gate 117 passed, 1 pre-existing manual keyring smoke ignored; no real credentials |
| Phase 5  | capability/vision/structured/unknown gate          | NOT RUN | —         | —                |
| Phase 6  | three-language/Fluent/reader shell                 | NOT RUN | —         | —                |
| Phase 7  | atomic credential/book→Key→read/AI services        | NOT RUN | —         | —                |
| Phase 8  | teaching instruction/revision/test no-history      | NOT RUN | —         | —                |
| Phase 9  | page index/resume/partial/correction/provenance    | NOT RUN | —         | —                |
| Phase 10 | Explorer library/drop/import/index status          | NOT RUN | —         | —                |
| Phase 11 | text/region anchors/preparation/notes              | NOT RUN | —         | —                |
| Phase 12 | registry/atomic history/multi-panel/hide continue  | NOT RUN | —         | —                |
| Phase 13 | delete/backup/restore/clear/privacy                | NOT RUN | —         | —                |
| Phase 14 | deterministic overview/book questions              | NOT RUN | —         | —                |
| Phase 15 | A–Q/clean Windows/package/release                  | NOT RUN | —         | —                |

## 3. Provider 能力证据

不在规划阶段预填不稳定事实。Phase 5 按当日官方来源填写；unknown 不等于 fail，也不允许 UI 使用。

| Provider/model               | text                               | image   | PDF/page | strict structured | limits verified        | official URL/date | contract result |
| ---------------------------- | ---------------------------------- | ------- | -------- | ----------------- | ---------------------- | ----------------- | --------------- |
| OpenAI / registry default    | PENDING                            | PENDING | PENDING  | PENDING           | PENDING                | —                 | NOT RUN         |
| Gemini / registry default    | PENDING                            | PENDING | PENDING  | PENDING           | PENDING                | —                 | NOT RUN         |
| Anthropic / registry default | PENDING                            | PENDING | PENDING  | PENDING           | PENDING                | —                 | NOT RUN         |
| DeepSeek / registry default  | PENDING                            | PENDING | PENDING  | PENDING           | PENDING                | —                 | NOT RUN         |
| Kimi / registry default      | PENDING                            | PENDING | PENDING  | PENDING           | PENDING                | —                 | NOT RUN         |
| Unknown/custom model rule    | text conservative after validation | DENY    | DENY     | DENY              | 32k text fallback only | ADR 0005          | NOT RUN         |

## 4. A–Q 验收

| ID  | 场景                            | 自动化 owner     | 必需手工补充                       | 状态    | Evidence |
| --- | ------------------------------- | ---------------- | ---------------------------------- | ------- | -------- |
| A   | book→Key→read                   | Phase 7/15       | clean keyring onboarding           | NOT RUN | —        |
| B   | zh-CN/zh-TW/en                  | Phase 6/15       | Windows locale/scale               | NOT RUN | —        |
| C   | teaching instruction            | Phase 8/15       | copy/usability review              | NOT RUN | —        |
| D   | direct text explanation         | Phase 11–12/15   | real reader selection              | NOT RUN | —        |
| E   | region containing reliable text | Phase 11/15      | three-format visual check          | NOT RUN | —        |
| F   | chart/image region confirmation | Phase 11–12/15   | verified vision provider           | NOT RUN | —        |
| G   | multiple floating panels        | Phase 12/15      | drag/resize/scale                  | NOT RUN | —        |
| H   | hidden panel continues          | Phase 12/15      | real window interaction            | NOT RUN | —        |
| I   | history continue/delete         | Phase 12/15      | restart UX                         | NOT RUN | —        |
| J   | decline scanned-PDF AI          | Phase 9/15       | network observation                | NOT RUN | —        |
| K   | AI-assisted local index         | Phase 9/15       | verified provider + offline search | NOT RUN | —        |
| L   | partial failure/retry           | Phase 9/15       | restart/recovery                   | NOT RUN | —        |
| M   | manual text/LaTeX correction    | Phase 9/15       | side-by-side usability             | NOT RUN | —        |
| N   | provenance distinction          | Phase 9/11/14/15 | citation copy review               | NOT RUN | —        |
| O   | responsive/fullscreen           | Phase 6/12/15    | Win10/11, 200%, high contrast      | NOT RUN | —        |
| P   | backup/restore no Key           | Phase 13/15      | second clean profile               | NOT RUN | —        |
| Q   | delete/clear/leak               | Phase 13/15      | credential/app-data residual scan  | NOT RUN | —        |

## 5. Race/restart matrix

| Boundary        | Required cases                                               | Owner                | Status          |
| --------------- | ------------------------------------------------------------ | -------------------- | --------------- |
| import          | copy/parse/index cancel and restart                          | inherited + Phase 15 | PENDING RECHECK |
| provider stream | cancel before headers/delta/terminal, EOF/malformed          | Phase 4R/5           | PHASE 4R PASS; PHASE 5 RECHECK |
| page index      | claim owner, late response, validate/commit, partial restart | Phase 9              | NOT RUN         |
| learning        | hide/unmount, cancel vs terminal vs DB commit, two requests  | Phase 12             | NOT RUN         |
| deletion        | trash before/after DB commit, remote cleanup fail            | Phase 13             | NOT RUN         |
| backup          | active work, output temp/rename failure                      | Phase 13             | NOT RUN         |
| restore         | corrupt archive, every swap fault, rollback/reopen           | Phase 13             | NOT RUN         |
| clear all       | key deletion/data swap/restart                               | Phase 13             | NOT RUN         |

## 6. Privacy artifacts

| Artifact                       | Forbidden sentinel scan                                              | Owner          | Status  |
| ------------------------------ | -------------------------------------------------------------------- | -------------- | ------- |
| logs / error DTO / diagnostics | Key, path, content, images, prompt, answer, instruction, vendor body | every phase/15 | PHASE 4R PASS; RECHECK |
| SQLite                         | Key/raw page image/provider body/incomplete stream                   | Phase 9/12/13  | NOT RUN |
| `.tlbackup`                    | all credentials/IDs, logs, scratch, remote handle, incomplete output | Phase 13/15    | NOT RUN |
| frontend bundle/source map     | credential/content fixtures, custom provider URL                     | Phase 15       | NOT RUN |
| Tauri resources/installer      | secrets/user data/unsafe fixtures                                    | Phase 15       | NOT RUN |

## 7. Global release commands

Record exact date, HEAD, exit result, test count and only non-sensitive warnings when executed:

| Command group                                   | Status  | Evidence |
| ----------------------------------------------- | ------- | -------- |
| `npm ci`, Chromium install                      | NOT RUN | —        |
| format/lint/typecheck/test/build/e2e            | NOT RUN | —        |
| generated/sensitive/licenses/fixtures           | PARTIAL | Phase 4R generated/sensitive PASS; licenses/fixtures NOT RUN |
| cargo fmt/clippy/test all-features `-j 1`       | PHASE 4R PASS | `e4d12c2`; fmt and clippy PASS; tests 117 passed, 1 ignored |
| cargo deny                                      | NOT RUN | —        |
| Tauri debug/no-bundle and release bundle        | NOT RUN | —        |
| clean Windows 10/11 install                     | NOT RUN | —        |
| five-provider text + verified capability manual | NOT RUN | —        |

## 8. Evidence update rules

- `PASS` 只在命令/场景实际完成且无必需工作剩余时填写。
- 环境/服务故障写 `BLOCKED` 和 safe code，不写 provider body。
- 一项 provider 能力官方未知写 `UNKNOWN/DENIED`，不是伪造 PASS/FAIL。
- 每次 checkpoint 记录最终 HEAD；测试中使用自制内容，只记录计数/状态。

## 9. Phase 4R normalized evidence

All commands below completed on 2026-08-03 against code HEAD `e4d12c2`. Tests used the checked-in
synthetic provider fixtures, loopback HTTP servers, and synthetic credential strings; no real
credential or provider network call was used.

| Command | Result | Normalized evidence |
| --- | --- | --- |
| `cargo test --manifest-path src-tauri/Cargo.toml provider_contract -j 1` | PASS | 8 passed; five validation routes; provider-specific visible/hidden/terminal vectors; 37 applicable no-completion cases; 15 deterministic cancellation scenarios (five providers × three boundaries) |
| `cargo test --manifest-path src-tauri/Cargo.toml ai::providers -j 1` | PASS | 25 passed |
| `cargo test --manifest-path src-tauri/Cargo.toml ai::stream -j 1` | PASS | 7 passed |
| `cargo fmt --manifest-path src-tauri/Cargo.toml -- --check` | PASS | no formatting diff |
| `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- -D warnings` | PASS | no warning or error |
| `cargo test --manifest-path src-tauri/Cargo.toml --all-features -j 1` | PASS | 117 passed; 0 failed; 1 ignored manual disposable Windows Credential Manager smoke test |
| `npm.cmd run check:generated` | PASS | binding export test: 1 passed |
| `npm.cmd run check:sensitive` | PASS | sensitive-file guard passed |

The shared contract also probes normalized 401/credential, model-missing, permission, rate/quota,
context, refusal, ambiguous Gemini 429, 5xx, and offline outcomes. Unique sentinels cover the Key,
system/user/assistant text, visible delta, nested vendor body, and Unicode error; none entered
`AiError` debug/display/source, the serialized `AppErrorDto`, or captured tracing. The Windows
linker printed localized import-library creation notices during tests; these were non-sensitive
warnings and did not affect results. PowerShell blocked the `npm.ps1` shim, so the equivalent
Windows executable `npm.cmd` ran both required npm gates successfully.
