# 離線本機模型

[English](local-models.md) · [简体中文](local-models.zh-CN.md) · [繁體中文](local-models.zh-TW.md)

在「設定 → AI 服務」選擇「自動連接本機 AI」；首次使用頁面也提供同一按鈕。TextbookLens 會探索已有的 Ollama 或 LM Studio，必要時啟動本機服務，尋找已下載的文字模型，並用一個簡短的合成問題測試模型。可用模型會註冊為獨立 profile，並選擇一個通過測試的文字預設模型。重複執行會更新既有 profile，不會產生重複項目。

支援的無認證本機服務不需要 API Key。應用程式不會安裝軟體、下載模型、使用雲端模型，也不會在本機請求失敗後回退到雲端。需要認證的服務會單獨報告，TextbookLens 不會修改其認證設定。

## 支援的流程

- **Ollama：** 從一般 Windows 安裝位置或 PATH 探索可執行檔；優先使用本機 `OLLAMA_HOST` 連接埠，否則使用 11434。服務停止時可啟動 `ollama serve`，再透過 `/api/tags` 和 `/api/show` 尋找已安裝完成模型。每次推理前都會拒絕雲端／遠端模型，並透過 `/api/chat` 按需載入本機模型。
- **LM Studio（實驗性，真實機器驗證延至 v0.2.1）：** 從 LM Studio 主目錄或 PATH 探索 `lms.exe` 和保存的本機服務連接埠（預設 1234）。應用程式可透過已安裝 CLI 啟動服務，讀取 `lms ls --llm --json`，排除其他裝置上的模型，並在需要時執行 `lms load`。支援 LM Link 的版本會使用 `--local`，傳送問題前再次確認實例位於本機。
- 每個模型擁有獨立的模型識別碼、上下文預算和數字回環連接埠。用「用於學習」切換文字模型。本機 profile 不建立或需要 Windows 認證。
- 本機文字模型支援選區提問、整本提問、續問和教學指令測試。Ollama 中繼資料聲明支援視覺的模型還可透過「用於視覺」單獨設為圖片區域問答模型。每次請求前都會重新檢查圖片能力；文字模型和遠端模型不會收到圖片。本版本不支援 LM Studio 視覺、本機結構化頁面索引或本機雲端檔案擷取。

視覺能力依 profile 保存並在重新啟動後恢復。每次請求接受一張 PNG/JPEG/WebP，編碼後不超過 2 MiB，任一邊不超過 2048 像素，解碼像素不超過 1,048,576。TextbookLens 會為圖片預留額外上下文。探索到的 Ollama 視覺模型最多使用 16,384 上下文 token，同時受模型聲明限制。重新連接會保留既有文字預設模型；視覺預設模型需要獨立選擇。

## 執行與安全邊界

推理軟體和模型檔案必須已安裝，電腦也需要足夠的 RAM/VRAM。首次載入模型可能需要數分鐘。執行環境缺失、沒有文字模型、需要認證或回答測試失敗都會顯示在連接結果中。

所有本機 HTTP 請求只使用 `127.0.0.1`，略過代理並拒絕重新導向。應用程式還會檢查模型本地性，因為 Ollama localhost 可能提供雲端模型，LM Studio 可能透過 LM Link 顯示遠端裝置。

## 參考與驗證

- [Ollama 模型清單](https://docs.ollama.com/api/tags)與[雲端／本機模型區別](https://docs.ollama.com/cloud)。
- [LM Studio CLI 模型清單](https://lmstudio.ai/docs/cli/local-models/ls)、[模型載入](https://lmstudio.ai/docs/cli/local-models/load)和[服務啟動](https://lmstudio.ai/docs/cli/serve/server-start)。

合成回環測試涵蓋本機／遠端過濾、無認證持久化與刪除、重複探索、串流終止和重新導向拒絕。0.2.0 在真實 Ollama 0.33.3 與 `deepseek-r1:8b` 上完成探索、註冊和完整文字回答，並以獨立連接埠驗證自動啟動。Ollama/Qwen3-VL 4B 的本機視覺傳輸可以執行，但密集經濟圖表曾給出事實錯誤，因此「執行成功」不能理解為「答案正確」。LM Studio 尚未完成真實機器端到端驗證。
