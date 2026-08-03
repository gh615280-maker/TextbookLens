# TextbookLens Phase 15：全量加固、A–Q 验收与 Windows 发布计划

**目标：** 对所有重规划功能执行发布级恢复、隐私、可访问性、视觉、安装和真实 provider 门禁，产出可复现的 Windows 11 x64 V1 安装包与完整证据。Windows 10 不在 V1 支持或验收范围内，不作兼容性声明。

**模式：** 严格模式。此阶段不添加新产品范围；发现缺陷回到拥有该合同的最早阶段修复并重新跑受影响 checkpoint。

**任务包与模型：** P15-A Tasks 1–3 Sol Max；P15-B Tasks 4–5 Sol Extra High；P15-C Tasks 6–7 Terra High；P15-D Tasks 8–9 Sol Max。发布写入单线；只读视觉/a11y/privacy/package review 可并行。

## 前置条件

- Phase 1–3 和重规划 Phase 4R–14 checkpoints 全部 PASS。
- 读取正式规格完成定义、新验证矩阵、所有 ADR 和本计划。
- 工作树逐项归属；无真实 Key/教材进入 fixtures/logs/commits。
- 真实 provider/clean Windows 11 x64 门禁使用用户明确提供的测试账户环境，结果只记 provider/model/status/time，不记请求内容或 Key。

## 全局约束

- 禁止以 release fix 为由放宽 redaction、TLS、capability、book isolation、completion、backup validation 或 migration constraints。
- 所有 destructive tests 在 TempDir/专用测试 profile；真实 data deletion 需要明确目标验证。
- 同一门禁失败两次停止盲重试，记录根因和归属。

---

### Task 1：加固 offline 与所有中断恢复

**Files:**

- Modify only owning services/tests: import, indexing, learning registry, maintenance startup recovery
- Create: `src-tauri/tests/interrupted_operations.rs`
- Create: `e2e/offline-recovery.spec.ts`

- [ ] offline startup still library/read/search/note/history/overview/backup；AI controls safe offline error。
- [ ] crash points：import copy/parse/index、page render/send/validate/commit、learning stream/DB commit、delete trash、backup temp、restore swap、clear intent。
- [ ] restart state never fabricates completed output, never loses committed page/conversation, never loops recovery。
- [ ] network returns during queued user action only resumes when product contract allows; no background upload from consent preference。

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --test interrupted_operations -j 1
npm run test:e2e -- e2e/offline-recovery.spec.ts
git add <exact recovery files and tests>
git commit -m "fix: harden interrupted operation recovery"
```

---

### Task 2：执行删除/恢复/迁移破坏性边界审计

**Files:**

- Modify only defects in maintenance/db/migrations
- Create: `src-tauri/tests/destructive_boundaries.rs`

- [ ] upgrade database from migrations 0001…0010 at every historical cutoff and reopen；no migration rewrite/hash drift。
- [ ] malicious paths/symlinks/reparse/locks/read-only disk/full disk/rename denial/antivirus-style transient failure。
- [ ] delete one book/clear all/restore never target source original, external backup, workspace, home/root or second book。
- [ ] remote cleanup unavailable leaves retry record without local data resurrection。
- [ ] document recovery guarantees and remaining unavoidable Windows file-lock errors。

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --test destructive_boundaries -j 1
cargo test --manifest-path src-tauri/Cargo.toml --test database_contract -j 1
git add <exact maintenance/migration tests and fixes>
git commit -m "fix: enforce destructive local data boundaries"
```

---

### Task 3：完整隐私、泄漏与供应链审计

**Files:**

- Modify: `scripts/check-sensitive-files.mjs`, tests
- Modify: `scripts/check-npm-licenses.mjs`, tests as required
- Create: `scripts/audit-release-artifacts.mjs`, test
- Create: `docs/security/privacy-audit.md`
- Modify redaction/diagnostic/backup tests only for defects

- [ ] sentinel classes：credential, credential key, source path, textbook text, page image/base64, prompt, teaching instruction, answer, vendor body, remote ID, user note。
- [ ] scan logs, diagnostics, backups, DB safe exports, frontend bundles/source maps, Tauri resources, installer, crash/error strings, fixture archives。
- [ ] inspect HTTP features/dependency tree: no cookie/SOCKS/native TLS/browser fetch/custom production host/telemetry/update channel without spec。
- [ ] npm/cargo licenses, advisories, exact pins, generated lockfiles, fixture licenses；document accepted unavoidable metadata only。

```powershell
npm run check:sensitive
npm run check:licenses
node scripts/audit-release-artifacts.mjs
cargo deny --manifest-path src-tauri/Cargo.toml check
git add scripts/check-sensitive-files.mjs <tests> scripts/check-npm-licenses.mjs <tests> scripts/audit-release-artifacts.mjs <test> docs/security/privacy-audit.md <exact fixes>
git commit -m "chore: audit release privacy and supply chain"
```

---

### Task 4：三语、可访问性、Fluent 与响应式发布审计

**Files:**

- Create: `e2e/accessibility-visual.spec.ts`
- Create: `docs/testing/visual-a11y-audit.md`
- Modify components/styles/catalogs only for defects

- [ ] every route/dialog/menu/panel/error in zh-CN/zh-TW/en；catalog key/placeholder/overflow/pseudo-long-string review。
- [ ] keyboard-only, focus traps/return, screen-reader names/live regions, landmarks, skip, high contrast, color independence, reduced motion。
- [ ] widths 320/480/768/1024/1440+, 125/150/200% scale, maximized/fullscreen, display resize; panel title always reachable。
- [ ] Windows 11 Mica effect is optional；high contrast、forced colors、transparency off 或 effect failure 时使用 opaque operational fallback，且功能不依赖透明效果。
- [ ] automated axe plus manual NVDA/Windows Narrator critical flow checklist；no real textbook screenshot。

```powershell
npm run test:e2e -- e2e/accessibility-visual.spec.ts
npm test
npm run build
git add e2e/accessibility-visual.spec.ts docs/testing/visual-a11y-audit.md <exact UI fixes>
git commit -m "fix: complete trilingual accessibility and visual audit"
```

---

### Task 5：自动化 A–Q 验收矩阵

**Files:**

- Create/Modify focused e2e files under `e2e/acceptance/`
- Create: `scripts/run-acceptance.ps1`
- Modify: `docs/testing/2026-08-03-textbooklens-replanned-verification-matrix.md`

- [ ] A book→Key→read and invalid validation retention。
- [ ] B three languages persistence/no content translation。
- [ ] C instruction new requests/test no history。
- [ ] D–F text direct/region text/region image confirmation。
- [ ] G–I multiple panels/hide continue/history/delete。
- [ ] J–N decline index/local index/partial retry/correction/provenance。
- [ ] O responsive/fullscreen。
- [ ] P backup/restore no Key。
- [ ] Q delete/clear/leak scan。
- [ ] each scenario isolated temp app data/loopback provider and captures deterministic evidence；no ordering dependency。

```powershell
powershell -ExecutionPolicy Bypass -File scripts/run-acceptance.ps1
git add e2e/acceptance scripts/run-acceptance.ps1 docs/testing/2026-08-03-textbooklens-replanned-verification-matrix.md
git commit -m "test: automate replanned V1 acceptance scenarios"
```

---

### Task 6：完成用户、贡献、安全、数据和 provider 文档

**Files:**

- Modify/Create: `README.md`, `CONTRIBUTING.md`
- Create: `docs/user-guide.md`, `docs/ai-services.md`, `docs/data-backup.md`, `docs/index-quality.md`
- Create/Modify: `SECURITY.md`, `THIRD_PARTY_NOTICES.md`, `LICENSES.md`
- Update ADR/protocol links only with verified official sources

- [ ] three-language essential user instructions or clearly routed localized in-app help；book import/key/visual consent/index correction/panels/backup/delete。
- [ ] exact local vs sent data table；backup includes sources/no keys；clear all includes credentials。
- [ ] provider capability dates and unknown model rule；no price promises。
- [ ] contributor migration/generated/fixture/redaction/workspace rules。
- [ ] all links/licenses/version commands verified；no stale old onboarding/right panel claims。

```powershell
rg -n "选择服务商.*导入|固定右|Request Preview|kimi-k2" README.md docs CONTRIBUTING.md SECURITY.md
npm run check:licenses
git add README.md CONTRIBUTING.md docs/user-guide.md docs/ai-services.md docs/data-backup.md docs/index-quality.md SECURITY.md THIRD_PARTY_NOTICES.md LICENSES.md <verified ADR changes>
git commit -m "docs: document the replanned local-first release"
```

---

### Task 7：固定 CI、reproducible build 和安装包

**Files:**

- Modify/Create CI workflows only if repository uses them
- Modify: `scripts/preflight.ps1`
- Modify: `package.json`, `src-tauri/tauri.conf.json`, icons/version metadata
- Create: `docs/release-checklist.md`

- [ ] clean checkout install with pinned Node/Rust/toolchain and lockfiles；no network after dependency cache where supported。
- [ ] CI jobs for frontend, Rust, generated, sensitive, licenses, fixtures, acceptance subset, Windows build。
- [ ] release bundle contains only allowlisted resources/migrations/provider registry/licenses/icons；audit script scans it。
- [ ] signing/update publication not claimed unless credentials/channel explicitly provided；unsigned local artifact labeled accurately。

```powershell
powershell -ExecutionPolicy Bypass -File scripts/preflight.ps1
npm ci
npm run tauri build
node scripts/audit-release-artifacts.mjs
git add <exact CI/preflight/config/release checklist files>
git commit -m "chore: make the Windows release reproducible"
```

---

### Task 8：clean Windows 11 x64 与真实五家 provider 手工门禁

**Files:**

- Modify only: `docs/testing/2026-08-03-textbooklens-replanned-verification-matrix.md`, `docs/release-checklist.md`

- [ ] clean Windows 11 x64 install/uninstall、first-run、three formats、restart/source delete、language、真实 125/150/200% system display scaling、high contrast/opaque fallback、narrow readability 和 fullscreen/Esc/focus；浏览器 zoom/emulation 不替代系统缩放证据。
- [ ] backup/restore into second clean profile; verify no keys and reconnect flow。
- [ ] each of five providers: validate configured current text model and one harmless self-made prompt; record success/error normalized only。
- [ ] for each capability marked visual/structured supported, run one self-made tiny image/page fixture; unsupported profiles show gate and make no request。
- [ ] delete/clear/uninstall residual scan per product promises；no credential residue after clear all。

No code commit unless a real defect is found; defect returns to owning task. Evidence commit:

```powershell
git add docs/testing/2026-08-03-textbooklens-replanned-verification-matrix.md docs/release-checklist.md
git commit -m "test: record clean Windows release gates"
```

---

### Task 9：最终 scope、workspace 与 release gate

**Files:**

- Modify: verification matrix/release checklist/changelog version docs only

- [ ] run full global commands from master plan from clean dependency install。
- [ ] `git status --short` every path attributed；no user protected files staged；`git diff --check`。
- [ ] compare implementation against formal spec/out-of-scope and old-task matrix；no old Task 9/fixed right panel/browser provider fetch/cloud scope。
- [ ] verify migrations immutable and fresh/upgrade DB both PASS。
- [ ] hash final installer, record tool versions/test counts/known nonblocking warnings；do not record secret/content。
- [ ] checkpoint commit only when no required work remains。

```powershell
git diff --check
git status --short
git add <only final verification/release documents>
git commit -m "chore: complete TextbookLens V1 release gate"
```

## Phase 15 / V1 完成检查点

- A–Q、clean Windows 11 x64、five-provider text and verified visual capabilities、install/restore/delete/privacy/a11y all PASS。
- release artifact reproducible and scanned；no keys/content in evidence。
- workspace protected；no Phase 16 or additional feature work begins without a new approved change package。
