# TextbookLens Phase 6：三语 i18n 与 Windows Fluent 应用框架实施计划

**目标：** 建立严格三语目录、即时持久切换、最终顶部导航、Fluent 系统视觉和极简阅读工具栏，同时保持 Phase 1–5 功能和 reader adapters 可运行。

**架构：** Rust/SQLite 保存唯一 UI language；React context 提供当前语言和强类型消息；路由层区分普通 shell、onboarding shell 和 reader shell；视觉使用 CSS tokens、系统媒体查询和经核实的 Tauri window effects，不把业务状态放入样式层。

**任务包与模型：** P6-A Tasks 1–2 Terra High；P6-B Tasks 3–4 Terra High；P6-C Tasks 5–6 Sol High（reader lifecycle/全局回归）。每包新对话；语言 DTO/catalog 与 shell writer 不并行。

## 前置条件

- Phase 5 checkpoint PASS，generated provider types 稳定。
- 读取正式规格第 5、9 节和本计划。
- 复用现有 `ReaderController`、PDF/EPUB/DOCX adapters、TOC/search/progress/settings；不重写解析器。
- 不实现 onboarding credential、AI service data、floating answers 或 AI indexing。

## 全局约束

- SQLite 是 `ui_language` 和 `first_reader_hint_completed` 唯一持久源；不使用 localStorage。
- 三种 catalog 键集合在编译/测试中完全一致；禁止运行时静默回退掩盖缺键。
- 不翻译教材标题、正文、用户输入、批注或历史。
- 当前导航、焦点、错误和状态不能只靠颜色。
- V1 平台和本阶段手工验收基线仅为 Windows 11 x64；Windows 10 不在支持或验收范围内，不作兼容性声明。
- Windows 11 的 Mica 是可选效果；high contrast、forced colors、透明关闭或效果失败时必须保留不透明且可操作的回退。
- reader shell 切换不能 dispose/recreate 当前 adapter，除非 book/format 真正改变。

---

### Task 1：收紧语言设置和首次提示迁移

**Files:**

- Create: `src-tauri/migrations/0006_replanned_ui_preferences.sql`
- Modify: `src-tauri/src/domain/settings.rs`
- Modify: `src-tauri/src/db/settings.rs`
- Modify: `src-tauri/src/commands/settings.rs`
- Modify: `src-tauri/src/commands/mod.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/tests/database_contract.rs`
- Modify: `src-tauri/tests/bindings.rs`
- Regenerate: `src/lib/generated/settings.ts`

**Interfaces:**

```rust
enum UiLanguage { ZhCn, ZhTw, En }
struct AppSettingsDto {
    ui_language: UiLanguage,
    ui_language_initialized: bool,
    first_reader_hint_completed: bool,
    // existing fields unchanged
}
async fn initialize_ui_language(detected: UiLanguage) -> AppSettingsDto;
async fn update_ui_language(language: UiLanguage) -> AppSettingsDto;
async fn complete_first_reader_hint() -> AppSettingsDto;
```

- [ ] 迁移用 table rebuild/check 只接受 `zh-CN|zh-TW|en`，保留 reader settings/profile FK；旧合法值原样，其他值映射 `en` 并设 initialized=false。
- [ ] 首次初始化是 compare-and-set：只在 `ui_language_initialized=0` 使用前端检测值；后续启动不覆盖用户选择。
- [ ] 严格 UTC/boolean/enum decode；并发两个初始化只有一个生效。
- [ ] binding JSON 精确为产品 locale 标签，不暴露 Rust variant 名。

```powershell
cargo test --manifest-path src-tauri/Cargo.toml settings
cargo test --manifest-path src-tauri/Cargo.toml --test database_contract
npm run check:generated
git add src-tauri/migrations/0006_replanned_ui_preferences.sql src-tauri/src/domain/settings.rs src-tauri/src/db/settings.rs src-tauri/src/commands/settings.rs src-tauri/src/commands/mod.rs src-tauri/src/lib.rs src-tauri/tests/database_contract.rs src-tauri/tests/bindings.rs src/lib/generated/settings.ts
git commit -m "feat: persist strict application language preferences"
```

---

### Task 2：建立强类型三语运行时和完整目录

**Files:**

- Modify: `src/lib/i18n.ts`
- Modify: `src/lib/i18n.test.ts`
- Modify: `src/lib/messages/zh-CN.ts`
- Create: `src/lib/messages/zh-TW.ts`
- Create: `src/lib/messages/en.ts`
- Create: `src/lib/locale.ts`
- Create: `src/lib/locale.test.ts`
- Create: `src/app/LanguageProvider.tsx`
- Create: `src/components/LanguageMenu.tsx`
- Modify: `src/app/AppProviders.tsx`
- Modify: `src/test/tauri-mock.ts`

- [ ] 用 `satisfies Record<MessageKey,string>` 保证 zh-TW/en 键与 zh-CN 完全相同；测试额外键、缺键、空字符串、未替换 placeholder。
- [ ] `detectWindowsLanguage(navigator.languages)`：含 Hant/TW/HK/MO → zh-TW；其他 zh → zh-CN；其余 → en。只用于首次初始化。
- [ ] LanguageProvider bootstrap SQLite 设置，切换时 optimistic UI 必须在 command 失败后回滚并播报安全错误。
- [ ] 语言菜单三项自称名称、不用旗帜，键盘/焦点/当前项语义正确。
- [ ] 将已有硬编码 `formatMessage('zh-CN',...)` 改为 hook；本任务覆盖所有当前 shell/placeholder 页，不提前写后续页面 copy。

```powershell
npm test -- src/lib/i18n.test.ts src/lib/locale.test.ts src/app/LanguageProvider.test.tsx
npm run typecheck
npm run lint
git add src/lib/i18n.ts src/lib/i18n.test.ts src/lib/messages/zh-CN.ts src/lib/messages/zh-TW.ts src/lib/messages/en.ts src/lib/locale.ts src/lib/locale.test.ts src/app/LanguageProvider.tsx src/components/LanguageMenu.tsx src/app/AppProviders.tsx src/test/tauri-mock.ts <converted existing components>
git commit -m "feat: add complete three-language application catalogs"
```

---

### Task 3：实现最终普通 shell 和路由边界

**Files:**

- Modify: `src/app/router.tsx`
- Modify: `src/components/AppLayout.tsx`
- Create: `src/components/AppShell.tsx`
- Create: `src/components/ReaderShell.tsx`
- Create: `src/components/OnboardingShell.tsx`
- Create: `src/components/TopNavigation.tsx`
- Create placeholders: `src/features/teaching/TeachingInstructionsPage.tsx`, `src/features/providers/AiServicesPage.tsx`
- Modify: `src/app/App.test.tsx`
- Create: `src/components/AppShell.test.tsx`

- [ ] 普通路由显示品牌、书库、教学指令、AI 服务、语言、设置；不显示 onboarding 作为主导航项。
- [ ] reader route 使用 ReaderShell，完全替换普通导航；onboarding route 使用简化 shell 但保留语言。
- [ ] 用 NavLink `aria-current=page`、skip link 和 route title；深链接/刷新保持正确 shell。
- [ ] placeholder 页面仅说明阶段未实现，不伪造 provider/指令状态；后续阶段原位替换。
- [ ] 键盘顺序、窄窗 overflow 和路由错误边界测试。

```powershell
npm test -- src/app/App.test.tsx src/components/AppShell.test.tsx
npm run typecheck
npm run lint
git add src/app/router.tsx src/components/AppLayout.tsx src/components/AppShell.tsx src/components/ReaderShell.tsx src/components/OnboardingShell.tsx src/components/TopNavigation.tsx src/features/teaching/TeachingInstructionsPage.tsx src/features/providers/AiServicesPage.tsx src/app/App.test.tsx src/components/AppShell.test.tsx
git commit -m "feat: add the replanned application shell"
```

---

### Task 4：实现 Fluent tokens、系统偏好和窗口回退

**Files:**

- Modify: `src/styles/tokens.css`
- Modify: `src/styles/themes.css`
- Modify: `src/styles/global.css`
- Create: `src/styles/fluent.css`
- Create: `src/lib/system-preferences.ts`
- Create: `src/lib/system-preferences.test.ts`
- Modify if official Tauri API supports it: `src-tauri/tauri.conf.json`, `src-tauri/capabilities/main.json`

- [ ] 定义 typography/space/radius/border/elevation/control-height/focus/accent/danger tokens；组件不得散落硬编码主题色。
- [ ] `prefers-color-scheme`、`forced-colors`、`prefers-reduced-motion` 和文字缩放测试；forced colors 禁用透明/阴影并保留 focus。
- [ ] 使用 CSS system colors/安全 fallback 跟随 accent；不可读取或记录系统敏感设置。
- [ ] 从官方 Tauri 文档验证 Mica window effect。Mica 仅是 Windows 11 上的可选增强，且只在非高对比、非 forced colors、透明开启时尝试；任何失败静默回退不透明纯色但写安全 diagnostic ID。
- [ ] 添加 Story-like test page 仅在测试环境或组件测试中覆盖按钮、输入、菜单、进度、危险状态，不向产品添加 demo route。

```powershell
npm test -- src/lib/system-preferences.test.ts src/components/AppShell.test.tsx
npm run format:check
npm run lint
npm run build
git add src/styles/tokens.css src/styles/themes.css src/styles/global.css src/styles/fluent.css src/lib/system-preferences.ts src/lib/system-preferences.test.ts <verified Tauri config files>
git commit -m "style: establish the Windows Fluent visual foundation"
```

---

### Task 5：替换三栏布局为极简阅读工具栏

**Files:**

- Modify: `src/features/reader/ReaderLayout.tsx`
- Modify: `src/features/reader/ReaderLayout.test.tsx`
- Modify: `src/features/reader/ReaderPage.tsx`
- Modify: `src/features/reader/ReaderPage.test.tsx`
- Modify: `src/features/reader/ReaderToolbar.tsx`
- Modify: `src/features/reader/ReaderPanel.tsx`
- Modify: `src/features/reader/ReaderSidebar.tsx`
- Modify: `src/features/reader/ReaderToc.tsx`
- Modify: `src/features/reader/ReaderSearch.tsx`
- Modify: `src/features/reader/ReaderSettingsControls.tsx`
- Create: `src/features/reader/ReaderFirstHint.tsx`
- Create: `src/features/reader/reader-fullscreen.ts`
- Modify: reader CSS files only where layout requires

- [ ] 工具栏精确包含返回书库、目录、教材/位置、搜索、框选占位动作、Aa、全屏；框选功能到 Phase 11 才启用。
- [ ] TOC/search/settings 按需 overlay/drawer，关闭不 dispose reader adapter；书/format 改变才执行 controller open/dispose。
- [ ] F11/toolbar full-screen 同步；Esc 先关闭最上层 transient UI；焦点返回触发按钮。
- [ ] first hint 只在设置 false 时显示，完成一次选择/框选或手动关闭后持久结束。
- [ ] 连续滚动和正文可读宽度在三格式保持；窄窗不丢工具栏主要动作。

```powershell
npm test -- src/features/reader/ReaderLayout.test.tsx src/features/reader/ReaderPage.test.tsx src/features/reader/ReaderController.test.ts
npm run typecheck
npm run lint
git add src/features/reader <only exact changed reader files>
git commit -m "feat: add the minimal Fluent reader shell"
```

---

### Task 6：Phase 6 checkpoint

**Files:**

- Modify: `e2e/shell.spec.ts`
- Create: `e2e/localization-reader-shell.spec.ts`
- Modify: `docs/testing/2026-08-03-textbooklens-replanned-verification-matrix.md`

- [ ] Playwright 覆盖三语即时切换/重启 mock、普通/onboarding/reader shell、keyboard skip、F11/escape、窄窗、高对比和 axe。
- [ ] 运行 104+ 既有 reader/import/frontend tests，确保 parser/controller 无回归。
- [ ] 全量前端门禁、generated、Rust settings tests、Tauri debug build。
- [ ] 使用自制内容取得真实 Windows 11 x64 手工证据：系统显示缩放实际设为 200%，验证 reader shell、high contrast/不透明回退、窄窗可读性、全屏/Esc/焦点返回；Mica 若未启用不阻塞。浏览器 zoom、设备模拟或强制媒体 emulation 不能替代系统缩放证据。记录并恢复原始外观设置，截图不含真实教材。

```powershell
npm run format:check
npm run lint
npm run typecheck
npm test
npm run build
npm run test:e2e
npm run check:generated
cargo test --manifest-path src-tauri/Cargo.toml settings -j 1
npm run tauri build -- --debug --no-bundle
git add e2e/shell.spec.ts e2e/localization-reader-shell.spec.ts docs/testing/2026-08-03-textbooklens-replanned-verification-matrix.md
git commit -m "chore: complete the Fluent shell checkpoint"
```

## Phase 6 完成检查点

- 三语目录/持久化/系统默认和全部 shell 通过。
- reader adapter 生命周期、位置、搜索、目录和设置不回归。
- 已保存可持久核验的 Windows 11 x64、真实 200% 系统显示缩放和 high contrast/不透明回退手工证据，且 focused regression 通过。
- 仍没有凭据写入 UI、AI 索引或浮动面板。
