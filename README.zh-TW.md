<div align="center">

![TextbookLens：本機優先的教材閱讀器](docs/assets/textbooklens-social-preview.jpg)

# TextbookLens

**在一個重視隱私的 Windows 應用程式中閱讀、搜尋、註解並向 PDF、EPUB 和 DOCX 教材提問。**

[![持續整合](https://github.com/gh615280-maker/TextbookLens/actions/workflows/ci.yml/badge.svg)](https://github.com/gh615280-maker/TextbookLens/actions/workflows/ci.yml)
[![最新版本](https://img.shields.io/github/v/release/gh615280-maker/TextbookLens?display_name=tag&sort=semver)](https://github.com/gh615280-maker/TextbookLens/releases/latest)
[![累計下載](https://img.shields.io/github/downloads/gh615280-maker/TextbookLens/total)](https://github.com/gh615280-maker/TextbookLens/releases)
[![開源授權](https://img.shields.io/github/license/gh615280-maker/TextbookLens)](LICENSE)

[**下載 Windows 安裝版**](https://github.com/gh615280-maker/TextbookLens/releases/latest) · [English](README.md) · [简体中文](README.zh-CN.md) · [繁體中文](README.zh-TW.md)

[使用手冊](docs/user-guide.zh-TW.md) · [AI 服務](docs/ai-services.zh-TW.md) · [本機模型](docs/local-models.zh-TW.md) · [開發路線](ROADMAP.md)

</div>

## 為什麼選擇 TextbookLens

- **統一教材庫：** PDF、EPUB、DOCX 不必再分別使用不同閱讀器。
- **持續的學習脈絡：** 教材、閱讀位置、筆記、註解和已完成問答都依書籍保存。
- **減少對話碎片：** 一般 AI 對話有上下文上限，長期學習往往要反覆建立新對話、上傳資料和說明背景。TextbookLens 會為每次提問擷取目前教材的相關內容。它不會消除模型的上下文上限，但能減少重複準備，讓學習記錄留在同一本書中。
- **本機優先：** 閱讀、本機搜尋、註解、歷程和備份保存在自己的電腦上。
- **AI 由你主動觸發：** 只有明確發起 AI 操作時，才會傳送該操作所需的有限內容。
- **離線本機 AI：** 一鍵連接已安裝的 Ollama 文字模型；相容的 Ollama 視覺模型可回答圖片區域問題。LM Studio 文字支援仍是實驗性功能。
- **原始碼完整開放：** Rust、React、Tauri 和 SQLite 原始碼採用 Apache-2.0 授權。

<div align="center">

![以合成測試資料展示的 TextbookLens 書庫介面](docs/assets/textbooklens-library.png)

<sub>真實應用程式介面，內容為合成測試資料。</sub>

</div>

## 下載即用

開啟[最新版本下載頁](https://github.com/gh615280-maker/TextbookLens/releases/latest)，依需求選擇：

- `TextbookLens_*_x64-setup.exe`：建議的標準小型安裝程式；使用電腦現有的 WebView2，缺少時需要連線下載。
- `TextbookLens_*_x64_en-US.msi`：適合集中部署的標準 MSI。
- 檔名含 `offline` 的套件：附帶 Microsoft WebView2，可在缺少執行階段且無法連線的電腦上安裝。
- `SHA256SUMS.txt`：四個安裝檔的 SHA-256 校驗值。

兩類安裝包都不包含 Ollama、LM Studio 或模型權重。安裝後直接匯入 PDF、EPUB 或 DOCX；AI 並非必要，需要時再連接本機執行環境或設定雲端服務。

> [!IMPORTANT]
> 目前支援 Windows 11 x64，安裝檔尚未進行程式碼簽章。Windows SmartScreen 可能顯示「未知發行者」，請在選擇「仍要執行」前先核對 SHA-256。

## 隱私邊界

TextbookLens 沒有帳號系統、雲端同步、內建 AI 額度、專案中轉伺服器、協作服務、遙測或靜默背景上傳。雲端 API 金鑰透過 Windows 認證管理員保存，不寫入應用程式資料庫、瀏覽器儲存、日誌、診斷檔或備份；本機 Ollama 和 LM Studio profile 不需要雲端金鑰。

匯入、閱讀、本機搜尋、註解、語言切換、備份和還原都不需要 AI 請求。準確邊界見 [AI 服務與傳送邊界](docs/ai-services.zh-TW.md)、[離線本機模型](docs/local-models.zh-TW.md)和[資料、備份、還原與刪除](docs/data-backup.zh-TW.md)。

## 開發者上手

```powershell
git clone https://github.com/gh615280-maker/TextbookLens.git
cd TextbookLens
npm ci
npm run tauri dev
```

固定工具鏈為 Node.js 24.18.1、npm 11.16.0、Rust 1.97.1，並需要 Tauri 2 的 Windows 桌面相依項目。`npm run preflight` 執行發行級檢查，`npm run tauri build` 產生本機 NSIS/MSI 安裝檔。

修改資料、認證、網路、遷移、測試夾具或自動產生綁定前，請閱讀英文 [CONTRIBUTING.md](CONTRIBUTING.md)。相依套件來源見英文 [LICENSES.md](LICENSES.md) 和 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)。

## 目前狀態

0.2.0 是目前的早期公開版本，加入一鍵發現 Ollama、離線文字問答、獨立 Ollama 視覺模型、PDF 解碼資源和多項閱讀器可靠性修正。前端、Rust、安全性、Windows 建置、安裝、離線安裝和 v0.1.0 升級檢查結果見英文 [v0.2.0 驗證記錄](docs/releases/v0.2.0-validation.md)。

已知限制包括：安裝檔未簽章、沒有自動更新、LM Studio 文字實機驗證延至 v0.2.1、沒有本機全書 OCR／視覺索引，以及本機視覺模型可能在困難材料上給出錯誤答案。詳情見 [v0.2.0 發行說明](docs/releases/v0.2.0.zh-TW.md)。

歡迎透過 [GitHub Issues](https://github.com/gh615280-maker/TextbookLens/issues) 提交建議或問題。如果專案對你有幫助，歡迎給一個 **Star**。
