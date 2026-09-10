<div align="center">

![TextbookLens：本地优先的教材阅读器](docs/assets/textbooklens-social-preview.jpg)

# TextbookLens

**在一个注重隐私的 Windows 应用中阅读、搜索、批注并向 PDF、EPUB 和 DOCX 教材提问。**

[![持续集成](https://github.com/gh615280-maker/TextbookLens/actions/workflows/ci.yml/badge.svg)](https://github.com/gh615280-maker/TextbookLens/actions/workflows/ci.yml)
[![最新版本](https://img.shields.io/github/v/release/gh615280-maker/TextbookLens?display_name=tag&sort=semver)](https://github.com/gh615280-maker/TextbookLens/releases/latest)
[![累计下载](https://img.shields.io/github/downloads/gh615280-maker/TextbookLens/total)](https://github.com/gh615280-maker/TextbookLens/releases)
[![开源协议](https://img.shields.io/github/license/gh615280-maker/TextbookLens)](LICENSE)

[**下载 Windows 安装版**](https://github.com/gh615280-maker/TextbookLens/releases/latest) · [English](README.md) · [简体中文](README.zh-CN.md) · [繁體中文](README.zh-TW.md)

[使用手册](docs/user-guide.zh-CN.md) · [AI 服务](docs/ai-services.zh-CN.md) · [本地模型](docs/local-models.zh-CN.md) · [开发路线](ROADMAP.md)

</div>

## 为什么选择 TextbookLens

- **统一教材库：** PDF、EPUB、DOCX 不再需要分别使用不同阅读器。
- **持续的学习上下文：** 教材、阅读位置、笔记、批注和已完成问答都按书保存。
- **减少对话碎片：** 普通 AI 对话存在上下文上限，长期学习往往要反复新建对话、上传资料和解释背景。TextbookLens 会为每次提问检索当前教材的相关内容。它不会消除模型的上下文上限，但能减少重复准备，让学习记录留在同一本书中。
- **本地优先：** 阅读、本地搜索、批注、历史和备份保存在自己的电脑上。
- **AI 由你主动触发：** 只有明确发起 AI 操作时，才会发送该操作所需的有限内容。
- **离线本地 AI：** 一键连接已经安装的 Ollama 文字模型；兼容的 Ollama 视觉模型可回答图片区域问题。LM Studio 文字支持仍为实验性功能。
- **源码完整开放：** Rust、React、Tauri 和 SQLite 源码采用 Apache-2.0 协议。

<div align="center">

![使用合成测试数据展示的 TextbookLens 书库界面](docs/assets/textbooklens-library.png)

<sub>真实应用界面，内容为合成测试数据。</sub>

</div>

## 下载即用

打开[最新版下载页](https://github.com/gh615280-maker/TextbookLens/releases/latest)，按需要选择：

- `TextbookLens_*_x64-setup.exe`：推荐的标准小安装包；复用电脑已有的 WebView2，缺少时需要联网下载。
- `TextbookLens_*_x64_en-US.msi`：用于集中部署的标准 MSI。
- 文件名含 `offline` 的包：附带微软 WebView2，可在缺少运行时且无法联网的电脑上安装。
- `SHA256SUMS.txt`：四个安装包的 SHA-256 校验值。

两类安装包都不包含 Ollama、LM Studio 或模型权重。安装后直接导入 PDF、EPUB 或 DOCX；AI 不是必需项，需要时再连接本地运行环境或配置云端服务。

> [!IMPORTANT]
> 当前支持 Windows 11 x64，安装包尚未进行代码签名。Windows SmartScreen 可能提示“未知发布者”，请选择“仍要运行”前先核对 SHA-256。

## 隐私边界

TextbookLens 没有账号系统、云同步、内置 AI 额度、项目中转服务器、协作服务、遥测或静默后台上传。云端 API 密钥通过 Windows 凭据管理器保存，不写入应用数据库、浏览器存储、日志、诊断文件或备份；本地 Ollama 和 LM Studio profile 不需要云端密钥。

导入、阅读、本地搜索、批注、语言切换、备份和恢复都不需要 AI 请求。准确边界见 [AI 服务与发送边界](docs/ai-services.zh-CN.md)、[离线本地模型](docs/local-models.zh-CN.md)和[数据、备份、恢复与删除](docs/data-backup.zh-CN.md)。

## 开发者上手

```powershell
git clone https://github.com/gh615280-maker/TextbookLens.git
cd TextbookLens
npm ci
npm run tauri dev
```

固定工具链为 Node.js 24.18.1、npm 11.16.0、Rust 1.97.1，并需要 Tauri 2 的 Windows 桌面依赖。`npm run preflight` 执行发布级检查，`npm run tauri build` 生成本地 NSIS/MSI 安装包。

修改数据、凭据、网络、迁移、测试夹具或自动生成绑定前，请阅读英文 [CONTRIBUTING.md](CONTRIBUTING.md)。依赖来源见英文 [LICENSES.md](LICENSES.md) 和 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)。

## 当前状态

0.2.0 是当前早期公开版本，加入一键发现 Ollama、离线文字问答、独立 Ollama 视觉模型、PDF 解码资源和多项阅读器可靠性修复。前端、Rust、安全、Windows 构建、安装、离线安装和 v0.1.0 升级检查结果见英文 [v0.2.0 验证记录](docs/releases/v0.2.0-validation.md)。

已知限制包括：安装包未签名、没有自动更新、LM Studio 文字实机验证留到 v0.2.1、没有本地全书 OCR/视觉索引，以及本地视觉模型可能在困难材料上给出错误答案。详情见 [v0.2.0 发布说明](docs/releases/v0.2.0.zh-CN.md)。

欢迎通过 [GitHub Issues](https://github.com/gh615280-maker/TextbookLens/issues) 提交建议或问题。如果项目对你有帮助，欢迎点一个 **Star**。
