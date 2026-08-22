# TextbookLens public launch kit

Use these drafts only after checking that the linked release is current and CI is green. Replace bracketed placeholders with real experience; do not invent testimonials, benchmarks, or user counts.

## Core positioning

TextbookLens is an open-source, local-first Windows textbook reader for PDF, EPUB, and DOCX. It keeps the library and learning data on the device, while optional AI actions run only when the user explicitly starts them.

- Repository: https://github.com/gh615280-maker/TextbookLens
- Download: https://github.com/gh615280-maker/TextbookLens/releases/latest
- Suggested visual: `docs/assets/textbooklens-social-preview.jpg`

## 小红书

**标题候选：**

1. 我做了一个把 PDF / EPUB / DOCX 放进同一个书库的开源阅读器
2. 不想把教材上传到云端，我做了一个本地优先的 Windows 阅读器
3. TextbookLens 第一版公开：下载即用，也可以拿源码随意改

**正文：**

最近把 TextbookLens 的第一版公开了。它是一个 Windows 11 本地优先教材阅读器，可以把 PDF、EPUB 和 DOCX 放在同一个书库里阅读、搜索和批注。

我最在意的是边界清楚：书库、进度、笔记、索引和备份保存在本机；普通阅读不需要 AI；只有你主动发起 AI 操作时才会发送相关内容。项目没有账号、云同步或中转服务器。

普通用户可以直接下载 EXE 安装，开发者也能拿到完整的 Rust + React + Tauri 源码继续改。现在还是早期版本，只支持 Windows 11 x64，而且安装包暂未商业签名。

下载和源码：[仓库链接]

如果你愿意试用，最希望听到的是：你平时读哪种格式、哪个步骤最麻烦、哪里会让你放弃继续用。

**标签建议：** #开源软件 #学习工具 #PDF阅读器 #效率工具 #Windows软件

## 知乎

**标题：** 为什么我做了一个本地优先的开源教材阅读器 TextbookLens？

**开头：**

教材阅读工具常见的割裂是：PDF、EPUB、DOCX 各用一个应用，笔记和进度散落在不同位置；一旦加入 AI，又很难判断哪些内容被发送到了哪里。TextbookLens 的第一版试图把这两个问题放进一个边界明确的 Windows 桌面应用中解决。

**建议结构：**

1. 真实痛点：格式割裂、长期学习资料难管理。
2. 产品取舍：本地书库、本地搜索、AI 必须由用户主动触发。
3. 技术实现：Tauri 2、Rust、React、SQLite，以及为什么网络和凭据边界放在 Rust 端。
4. 已完成与未完成：支持的格式、备份恢复；Windows 11 x64、未签名等限制。
5. 开源邀请：普通用户下载试用，开发者查看路线图和贡献指南。

结尾链接：[仓库链接] / [下载链接]

## V2EX

**节点与标题：** 分享创造｜[开源] TextbookLens：本地优先的 PDF / EPUB / DOCX 教材阅读器

**正文：**

做了一个 Windows 11 x64 桌面教材阅读器 TextbookLens，目前第一版已开源。

主要特点：

- PDF / EPUB / DOCX 统一书库
- 阅读、搜索、批注、历史、备份与恢复
- 书库和学习数据默认留在本机
- AI 是可选能力，只有主动操作才发送相关内容
- Rust + React + Tauri 2，Apache-2.0

下载页：[下载链接]
源码：[仓库链接]

当前安装包未签名，SmartScreen 可能提示未知发布者；发布页提供 SHA256SUMS。更希望得到具体的复现步骤和工作流反馈，而不是泛泛的功能清单。

## Reddit

**Suggested communities:** r/opensource, r/SideProject, and a relevant reading/productivity community after checking each community's current self-promotion rules.

**Title:** I built an open-source, local-first textbook reader for PDF, EPUB, and DOCX on Windows

**Post:**

TextbookLens is a Windows 11 desktop reader that keeps PDF, EPUB, and DOCX books in one local library. Reading progress, notes, indexes, and backups stay on the device. AI is optional and sends content only after an explicit user action; there is no account, cloud sync, or project proxy.

The full Rust + React + Tauri source is Apache-2.0 licensed, and the release includes an EXE for non-developers. This is an early release: Windows 11 x64 only, and the installer is currently unsigned.

Repository: [repository link]
Download: [release link]

I would especially value feedback on import/reading workflows and reproducible Windows issues.

## Show HN

**Title:** Show HN: TextbookLens – A local-first textbook reader for PDF, EPUB, and DOCX

**Post:**

I built TextbookLens because textbook workflows are often split across format-specific readers, while AI-enabled tools can make data boundaries difficult to understand.

TextbookLens keeps the authoritative library, progress, notes, indexes, and backups on the Windows device. PDF, EPUB, and DOCX reading works without AI. Optional AI actions are explicit, and provider networking and credentials remain behind the Rust desktop boundary.

It is built with Tauri 2, Rust, React, and SQLite and released under Apache-2.0. The repository includes the full source, design/security documentation, deterministic synthetic fixtures, and downloadable Windows installers.

Repository: [repository link]

Limitations: Windows 11 x64 only today, unsigned installer, no cloud sync or accounts. I would appreciate technical review of the local-first boundary and feedback from people with long-form study workflows.
