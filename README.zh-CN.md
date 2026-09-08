<div align="center">

![TextbookLens：本地优先的教材阅读器](docs/assets/textbooklens-social-preview.jpg)

# TextbookLens

**在一个注重隐私的 Windows 应用中阅读 PDF、EPUB 和 DOCX 教材。**

[![持续集成](https://github.com/gh615280-maker/TextbookLens/actions/workflows/ci.yml/badge.svg)](https://github.com/gh615280-maker/TextbookLens/actions/workflows/ci.yml)
[![最新版本](https://img.shields.io/github/v/release/gh615280-maker/TextbookLens?display_name=tag&sort=semver)](https://github.com/gh615280-maker/TextbookLens/releases/latest)
[![累计下载](https://img.shields.io/github/downloads/gh615280-maker/TextbookLens/total)](https://github.com/gh615280-maker/TextbookLens/releases)
[![开源协议](https://img.shields.io/github/license/gh615280-maker/TextbookLens)](LICENSE)

[**下载 Windows 安装版**](https://github.com/gh615280-maker/TextbookLens/releases/latest) · [English](README.md) · [使用手册](docs/user-guide.md) · [开发路线](ROADMAP.md)

</div>

## 它解决什么问题

- **一本地书库：** PDF、EPUB、DOCX 不再需要分别使用不同阅读器。
- **本地优先：** 书库、阅读进度、笔记、索引和备份保存在你的电脑中。
- **AI 由你主动触发：** 普通阅读和本地搜索不需要 AI；只有你明确发起 AI 操作时才会发送相关内容。
- **离线 AI 问答：** 已安装 Ollama 并下载模型后，可一键连接本地文字模型；支持识图的模型还可用于图片区域问答。LM Studio 文字通道暂为实验性，实机验证安排在 v0.2.1。
- **面向系统学习：** 本地搜索、批注、阅读历史、多语言界面、备份与恢复集中在一个桌面工作流中。
- **源码完整开放：** Rust + React + Tauri 全部源码采用 Apache-2.0 协议，开发者可以自由修改和改进。

<div align="center">

![使用合成测试数据展示的 TextbookLens 书库界面](docs/assets/textbooklens-library.png)

<sub>真实应用界面，内容为合成测试数据。</sub>

</div>

## 下载即用

1. 打开 [最新版下载页](https://github.com/gh615280-maker/TextbookLens/releases/latest)。
2. 普通用户下载 `TextbookLens_*_x64-setup.exe`；需要集中部署时也可以选择 MSI。
3. 如需校验文件，可使用同一发布页中的 `SHA256SUMS.txt`。
4. 安装并启动，然后选择 PDF、EPUB 或 DOCX 文件。

> [!IMPORTANT]
> 当前只支持 Windows 11 x64，安装包尚未进行商业代码签名，因此 Windows SmartScreen 可能提示“未知发布者”。建议先核对 SHA-256，再选择“仍要运行”。

## 隐私边界

TextbookLens 没有账号系统、云同步、项目中转服务器、协作服务或内置 AI 额度。API 密钥交给 Windows 凭据管理器保存，不写入应用数据库、浏览器存储、日志、诊断文件或备份。

导入、阅读、本地搜索、批注、语言切换、备份和恢复都不需要发起 AI 请求。更精确的数据发送规则见 [AI 服务与发送边界](docs/ai-services.md)，存储、删除和恢复机制见 [数据与备份说明](docs/data-backup.md)。

## 开发者上手

项目技术栈为 React 19、TypeScript、Tauri 2、Rust 和 SQLite：

```powershell
git clone https://github.com/gh615280-maker/TextbookLens.git
cd TextbookLens
npm ci
npm run tauri dev
```

固定开发工具链为 Node.js 24.18.1、npm 11.16.0、Rust 1.97.1，并需要 Tauri 2 的 Windows 桌面构建依赖。`npm run preflight` 用于发布级检查，`npm run tauri build` 可在本地生成 NSIS/MSI 安装包。

修改数据、凭据、网络、测试夹具或自动生成绑定前，请先阅读 [贡献指南](CONTRIBUTING.md)。欢迎通过 [Issues](https://github.com/gh615280-maker/TextbookLens/issues) 报告问题或提出功能建议。

## 当前状态

TextbookLens 仍是早期公开版本。0.1.0 已通过有记录的本地未签名 V1 发布检查；代码签名、自动更新以及少量人工无障碍和服务商检查尚未完成。准确范围见 [发布检查清单](docs/release-checklist.md)。

如果它对你有帮助，欢迎点一个 **Star**，让更多读者和开发者看到它。
