# TextbookLens user guide

TextbookLens is a local-first Windows 11 x64 textbook reader. The application interface is available in `zh-CN`, `zh-TW`, and `en`; changing the interface language does not translate your textbook, notes, questions, or answers.

## Essential instructions / 基本操作 / 基本操作

### English

1. In **Library**, choose **Import** or drop a PDF, EPUB, or DOCX file. The app immediately copies it into its own local data area, then parses and indexes it.
2. The first-run sequence is **choose one book → add an API key → start reading**. In **AI services**, select a provider, paste a key, and choose **Validate and connect**. A failed validation keeps your book; the same page can replace or remove a key/profile.
3. Text selection can explain, give examples, derive steps, translate, ask, or create a local note. Normal text actions show a non-blocking sending summary, not a blocking request preview.
4. Use **Box select** for a region. Reliable text stays local. A region needing an image shows a first-send confirmation with provider, model, content type, image/page count, and cost risk. Cancel means no staging, request, or marker. “Do not ask again” is profile/category-scoped and never allows background upload.
5. A finished answer becomes a marker and floating panel. Closing hides the panel; it does not stop generation. Choose **Stop** to cancel only that request. Reopen from its marker, continue with the current learning profile, or delete it from history.
6. A book’s **learning overview** is local. When full-text preparation is ready, book-level questions use current-book retrieval, not the whole book.
7. Kimi detects China/international automatically with a no-content model probe; there is no region picker. Kimi **full-text preparation**, the exceptional-page **index quality** route, provenance/correction choices, backup/restore, delete-one-book, and Clear all data are described below.

### 简体中文

1. 在“书库”点击“导入”，或拖入 PDF、EPUB、DOCX。应用会先复制到自己的本地数据区，再解析和建立本地索引。
2. 首次流程是“选择一本书 → 添加 API Key → 开始阅读”。在“AI 服务”选择服务商、粘贴 Key，然后点“验证并连接”。验证失败不会删除已导入教材；同页可替换或删除 Key/profile。
3. 正常阅读、搜索和个人批注都在本地。选中文本后可解释、举例、逐步推导、翻译、提问或记批注；普通文本操作显示非阻断发送摘要。
4. 用“框选”选择公式、图表或区域。若需要发送图像，首次会显示服务商、模型、内容类型、图像/页数和费用风险；取消则不会暂存、发送或创建标记。选择“不再提示”只对当前 profile 的当前类别有效，绝不授权后台上传。
5. 完成的回答会保存为标记和浮动面板。关闭面板只是隐藏；只有“停止”会取消该请求。可从标记恢复、续问或在历史中删除。
6. 教材“学习概览”显示本地来源/状态汇总；全文准备完成后才可整本提问，且只检索本书相关本地片段。
7. Kimi 会用不含教材内容的模型探测自动识别中国/国际区域，没有区域选择器。下文说明 Kimi“全文准备”、异常页“索引质量”、来源/修正选择、备份/恢复、删书和清除全部数据。

### 繁體中文

1. 在「書庫」按「匯入」，或拖入 PDF、EPUB、DOCX。應用程式會先複製到自己的本機資料區，再解析和建立本機索引。
2. 首次流程是「選擇一本書 → 新增 API Key → 開始閱讀」。在「AI 服務」選服務商、貼上 Key，然後按「驗證並連線」。驗證失敗不會刪除已匯入教材；同一頁可替換或刪除 Key/profile。
3. 正常閱讀、搜尋和個人註解都在本機。選取文字後可解釋、舉例、逐步推導、翻譯、提問或記註解；一般文字操作會顯示非阻斷傳送摘要。
4. 用「框選」選擇公式、圖表或區域。若需要傳送影像，首次會顯示服務商、模型、內容類型、影像/頁數和費用風險；取消就不會暫存、傳送或建立標記。選「不再提示」只限目前 profile 的目前類別，絕不授權背景上傳。
5. 完成的回答會儲存為標記和浮動面板。關閉面板只是隱藏；只有「停止」會取消該請求。可從標記恢復、續問或在歷程中刪除。
6. 教材「學習概覽」顯示本機來源/狀態彙總；全文準備完成後才可整本提問，而且只擷取本書相關本機片段。
7. Kimi 會用不含教材內容的模型探測自動辨識中國/國際區域，沒有區域選擇器。下文說明 Kimi「全文準備」、異常頁「索引品質」、來源/修正選擇、備份/還原、刪書和清除所有資料。

## Local data and data sent to AI

| Data or action                                              | Local behavior                                                                    | Sent only after an explicit action?     |
| ----------------------------------------------------------- | --------------------------------------------------------------------------------- | --------------------------------------- |
| Imported copy, normalized sections, FTS, reading position   | App-owned local data and SQLite                                                   | No                                      |
| Reader, search, notes, markers, completed history, language | Local and available offline                                                       | No                                      |
| Settings, instruction, profile metadata, consent            | Local; keys are in Windows Credential Manager                                     | No                                      |
| Selected text/follow-up                                     | Frozen selection, needed same-book context/history, current instruction, question | Yes — learning action                   |
| Teaching-instruction test                                   | Temporary; creates no book history                                                | Yes — test action                       |
| Region/page image                                           | Bounded capture remains local before confirmation                                 | Yes — confirmed visual action           |
| AI-assisted PDF indexing                                    | Local detection/rendering; local validation/commit                                | Yes — confirmed exceptional-page action |
| Kimi Files full-text preparation                            | PDF remains local until Kimi region binding completes; result/FTS remain local    | Yes — explicit preparation              |
| Book-level question                                         | Local overview; relevant current-book retrieval rather than the whole book        | Yes — book-question action              |

Provider/model metadata, selected profile, consent choice, and the current instruction may accompany the operation that needs them. TextbookLens does not promise provider retention, deletion, price, quota, or future availability.

## Kimi, index quality, and data operations

Kimi has no region picker. A content-free model probe binds the key to China or international before a Files upload. Chat, Files upload/polling/download, and cleanup use that region; replacement detects again. Only an explicit authentication/region mismatch tries the other region—timeouts, network/rate-limit, and server errors do not. Full-text preparation is distinct from optional exceptional-page indexing.

For a scanned/low-quality PDF, you can decline AI indexing and still read. Approved indexing sends only exceptional pages in small batches; each page is validated and committed locally. `partial` or `needs_review` books remain readable. `local_text`, `ai_transcribed`, `ai_description`, and `user_corrected` remain distinct; descriptions are not quotations. Corrections are local overlays and re-indexing must ask whether to retain, accept, or compare; it never silently overwrites them. See [Index quality](index-quality.md).

See [Data, backup, restore, and deletion](data-backup.md) before moving a backup. Deleting one book removes only its app-owned copy and associated data, not the source file. **Clear all data** is different: it removes TextbookLens local data and credentials after exact confirmation, and credential cleanup can require an explicit retry. It does not delete originals or separately saved `.tlbackup` files. Uninstall is not documented as a data-deletion guarantee and has not been release-validated.
