# 离线本地模型

[English](local-models.md) · [简体中文](local-models.zh-CN.md) · [繁體中文](local-models.zh-TW.md)

在“设置 → AI 服务”选择“自动连接本地 AI”；首次使用页面也提供同一按钮。TextbookLens 会发现已有的 Ollama 或 LM Studio，必要时启动本地服务，查找已下载的文字模型，并用一个简短的合成问题测试模型。可用模型会注册为独立 profile，并选择一个通过测试的文字默认模型。重复执行会更新已有 profile，不会制造重复项。

支持的无认证本地服务不需要 API Key。应用不会安装软件、下载模型、使用云模型，也不会在本地请求失败后回退到云端。需要认证的服务会单独报告，TextbookLens 不会修改其认证设置。

## 支持的流程

- **Ollama：** 从常规 Windows 安装位置或 PATH 发现可执行文件；优先使用本地 `OLLAMA_HOST` 端口，否则使用 11434。服务停止时可启动 `ollama serve`，再通过 `/api/tags` 和 `/api/show` 查找已安装完成模型。每次推理前都会拒绝云端/远端模型，并通过 `/api/chat` 按需加载本地模型。
- **LM Studio（实验性，真实机器验证留到 v0.2.1）：** 从 LM Studio 主目录或 PATH 发现 `lms.exe` 和保存的本地服务端口（默认 1234）。应用可通过已安装 CLI 启动服务，读取 `lms ls --llm --json`，排除其他设备上的模型，并在需要时执行 `lms load`。支持 LM Link 的版本会使用 `--local`，发送问题前再次确认实例为本机。
- 每个模型拥有独立的模型标识、上下文预算和数字回环端口。用“用于学习”切换文字模型。本地 profile 不创建或需要 Windows 凭据。
- 本地文字模型支持选区提问、整本提问、续问和教学指令测试。Ollama 元数据声明支持视觉的模型还可以通过“用于视觉”单独设为图片区域问答模型。每次请求前都会重新检查图片能力；文字模型和远端模型不会收到图片。本版本不支持 LM Studio 视觉、本地结构化页面索引或本地云文件提取。

视觉能力按 profile 保存并在重启后恢复。每次请求接受一张 PNG/JPEG/WebP，编码后不超过 2 MiB，任一边不超过 2048 像素，解码像素不超过 1,048,576。TextbookLens 会为图片预留额外上下文。发现的 Ollama 视觉模型最多使用 16,384 上下文 token，同时受模型声明限制。重新连接会保留已有文字默认模型；视觉默认模型需要独立选择。

## 运行与安全边界

推理软件和模型文件必须已经安装，电脑也需要足够的 RAM/VRAM。首次加载模型可能需要数分钟。运行环境缺失、没有文字模型、需要认证或回答测试失败都会显示在连接结果中。

所有本地 HTTP 请求只使用 `127.0.0.1`，绕过代理并拒绝重定向。应用还会检查模型本地性，因为 Ollama localhost 可能提供云模型，LM Studio 可能通过 LM Link 显示远端设备。

## 参考与验证

- [Ollama 模型列表](https://docs.ollama.com/api/tags)与[云端/本地模型区别](https://docs.ollama.com/cloud)。
- [LM Studio CLI 模型列表](https://lmstudio.ai/docs/cli/local-models/ls)、[模型加载](https://lmstudio.ai/docs/cli/local-models/load)和[服务启动](https://lmstudio.ai/docs/cli/serve/server-start)。

合成回环测试覆盖本地/远端过滤、无凭据持久化与删除、重复发现、流终止和重定向拒绝。0.2.0 在真实 Ollama 0.33.3 与 `deepseek-r1:8b` 上完成发现、注册和完整文字回答，并用独立端口验证自动启动。Ollama/Qwen3-VL 4B 的本地视觉传输可运行，但密集经济图表曾给出事实错误，因此“运行成功”不能理解为“答案正确”。LM Studio 尚未完成真实机器端到端验证。
