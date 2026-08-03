# ADR 0004: Fixed first-party provider protocol matrix

- Status: Accepted for Phase 4 Task 1
- Last verified: 2026-08-03
- Scope: protocol facts only; no adapter or transport is implemented by this ADR.

## Decision

TextbookLens uses only the production hosts below. The Rust-owned transport will inject a
localhost endpoint only for tests; production hosts, proxies, and browser `fetch` are not
configurable. Validation checks model visibility with the credential before any credential is
persisted. Generation operations use explicit streaming completion signals; EOF alone is never
success. Hidden reasoning is discarded and never crosses IPC.

The checked-in registry is a capability hint, not an allowlist or price table. The application
default output reservation is 4,096 tokens for every listed model; `vendor maximum` is verified
internally during registry validation and is not exposed as pricing or a user promise.

OpenAI protocol facts were re-verified on 2026-08-02. Docs MCP was unavailable, so the official
OpenAI web fallback was used; the current reference still matches the planned endpoint, request,
visible-delta, usage, and explicit-completion contracts.

Gemini protocol facts were re-verified on 2026-08-03 from the official Google API reference.

| Provider | Production host | Validation operation | Generation operation | Required headers / request mapping | Visible delta / usage / completion | Ignore safely | Official sources |
| --- | --- | --- | --- | --- | --- | --- | --- |
| OpenAI | `https://api.openai.com` | `GET /v1/models/{model}` | `POST /v1/responses` with `stream: true` | `Authorization` with the `Bearer` scheme (secret injected only at send time), `Content-Type: application/json`; map model and normalized input to `model` and `input`. | `response.output_text.delta` / final response `usage` / `response.completed`. | reasoning, tool, audio, image, and unknown extension events. | [retrieve model](https://developers.openai.com/api/reference/resources/models/methods/retrieve), [create response](https://developers.openai.com/api/reference/resources/responses/methods/create), [Responses streaming events](https://developers.openai.com/api/reference/resources/responses/streaming-events), [GPT-5.6 Sol](https://developers.openai.com/api/docs/models/gpt-5.6-sol) |
| Gemini | `https://generativelanguage.googleapis.com` | Empty-body `GET /v1beta/models/{percent-encoded model path segment}`; require positive token limits and `supportedGenerationMethods` containing `generateContent`. | `POST /v1beta/models/{encoded model}:streamGenerateContent?alt=sse` | Header-only `x-goog-api-key: <key>` and `Content-Type: application/json`; despite some Google examples using `?key=`, this application follows the API overview's header form. System is `systemInstruction`; adjacent user/model turns are coalesced, then history must start with user and strictly alternate; output limit is `generationConfig.maxOutputTokens`. | Candidate array element zero with `index` zero only; nonempty visible `content.parts[].text` / `usageMetadata.promptTokenCount` and `candidatesTokenCount` / exactly `STOP` or `MAX_TOKENS`. Prompt block, no candidate, malformed parts, no visible text, or any other finish reason fails. | Unknown additions; thought, encrypted/internal thought, and `thoughtSignature` never leave the adapter. Safety metadata is not emitted, but its block/finish outcome is enforced. An SSE `error` frame is only a parser-robustness vector, not claimed as a Gemini event type. | [model metadata API](https://ai.google.dev/api/models), [generate-content API](https://ai.google.dev/api/generate-content), [API overview](https://ai.google.dev/api), [Gemini 3 thought signatures](https://ai.google.dev/gemini-api/docs/generate-content/gemini-3), [troubleshooting](https://ai.google.dev/gemini-api/docs/troubleshooting) |
| Anthropic | `https://api.anthropic.com` | `GET /v1/models/{model}` | `POST /v1/messages` with `stream: true` | `x-api-key: <key>`, `anthropic-version: 2023-06-01`, `content-type: application/json`; map system text to `system`, messages to `messages`, and output limit to `max_tokens`. | `content_block_delta` with `text_delta` / `message_delta.usage` / `message_stop`. | `ping`, tool/input JSON, thinking, signature, and unknown event types. | [models](https://platform.claude.com/docs/en/about-claude/models/overview), [Messages](https://platform.claude.com/docs/en/api/messages/create), [streaming](https://platform.claude.com/docs/en/build-with-claude/streaming), [versioning](https://platform.claude.com/docs/en/api/versioning) |
| DeepSeek | `https://api.deepseek.com` | `GET /models`, then require the selected ID in the returned model list | `POST /chat/completions` with `stream: true`, using the OpenAI-format API | `Authorization` with the `Bearer` scheme (secret injected only at send time), `Content-Type: application/json`; map normalized messages to `messages`, output limit to `max_tokens`, and model to `model`. | `choices[].delta.content` / final `usage` stream chunk / `choices[].finish_reason`. | `reasoning_content`, tool calls, logprobs, and unknown fields. | [API quick start](https://api-docs.deepseek.com/), [chat completions](https://api-docs.deepseek.com/api/create-chat-completion/), [current model facts](https://api-docs.deepseek.com/quick_start/pricing/) |
| Kimi | `https://api.moonshot.ai/v1` | `GET /models`, then require the selected ID in the returned model list | `POST /chat/completions` with `stream: true` | `Authorization` with the `Bearer` scheme (secret injected only at send time), `Content-Type: application/json`; map normalized messages to `messages`, output limit to `max_completion_tokens`, and model to `model`. | `choices[].delta.content` / final `usage` stream chunk / `choices[].finish_reason`. | `reasoning_content`, tool calls, and unknown fields. | [model list](https://platform.kimi.ai/docs/models), [Kimi K3](https://platform.kimi.ai/docs/guide/kimi-k3-quickstart), [chat API](https://platform.kimi.ai/docs/api/chat) |

## Registry verification record

| Provider | Registry default model | Context window | Vendor maximum output | Registry default output | Verification and baseline delta |
| --- | ---: | ---: | ---: | ---: | --- |
| OpenAI | `gpt-5.6` | 1,050,000 | 128,000 | 4,096 | Verified from the [official GPT-5.6 Sol page](https://developers.openai.com/api/docs/models/gpt-5.6-sol); no delta from the 2026-08-01 plan. |
| Gemini | `gemini-3.6-flash` | 1,048,576 | 65,536 | 4,096 | Verified from the [official Gemini 3.6 Flash page](https://ai.google.dev/gemini-api/docs/models/gemini-3.6-flash); no delta. |
| Anthropic | `claude-sonnet-5` | 1,000,000 | 128,000 | 4,096 | Verified from the [official models overview](https://platform.claude.com/docs/en/about-claude/models/overview); no delta. |
| DeepSeek | `deepseek-v4-flash` | 1,000,000 | 393,216 | 4,096 | Verified from the [official model details](https://api-docs.deepseek.com/quick_start/pricing/); no delta. |
| Kimi | `kimi-k3` | 1,048,576 | 1,048,576 | 4,096 | The official [Kimi model list](https://platform.kimi.ai/docs/models) identifies K3 as the current flagship and the [K3 limits](https://platform.kimi.ai/docs/guide/kimi-k3-quickstart) state a 1M context and max completion limit of 1,048,576. This replaces the plan baseline `kimi-k2.6` (256K context; 32K documented K2.6 request default). |

The Kimi replacement is intentionally limited to the registry default. Users remain able to enter
any model ID. An unknown model always uses a 32,000-token input fallback unless the user explicitly
supplies another positive context limit.

## Streaming completion policies (verified 2026-08-03)

Anthropic may send more than one `message_delta`; TextbookLens accepts cumulative intermediate
deltas but emits output usage only from the final accepted delta. Completion requires a visible
text block, an `end_turn` or `stop_sequence` terminal reason, and `message_stop`. `tool_use`,
`refusal`, `model_context_window_exceeded`, `max_tokens`, and `pause_turn` are not persisted as a
completed answer.

DeepSeek's documented `thinking` control is `{"thinking":{"type":"disabled"}}`; its
stream request also sets `stream_options.include_usage: true`. A usage-only chunk has empty
`choices` and precedes `data: [DONE]`. DeepSeek documents `stop`, `length`,
`content_filter`, `tool_calls`, and `insufficient_system_resource` finish reasons. TextbookLens
uses the narrower product success policy `stop` plus `[DONE]`, so truncated, filtered, tool, or
interrupted output cannot be persisted as a completed answer. `reasoning_content` is discarded.
This is an application policy, not a claim that the other documented finish reasons are invalid.
The model-list response establishes credential-scoped exact `data[].id` membership only.
([list models](https://api-docs.deepseek.com/api/list-models),
[chat completions](https://api-docs.deepseek.com/api/create-chat-completion),
[thinking mode](https://api-docs.deepseek.com/guides/thinking_mode),
[error codes](https://api-docs.deepseek.com/quick_start/error_codes/))

Kimi K3 uses `https://api.moonshot.ai/v1`, exact `GET /models` membership, and
`POST /chat/completions`. Its request uses `max_completion_tokens`, `stream: true`, and
`stream_options.include_usage: true`; `max_tokens` is deprecated. K3 always has thinking enabled,
so the adapter does **not** send a fictional thinking-disable field. It requests
`reasoning_effort: "low"` and permanently discards `reasoning_content`; only candidate zero
`delta.content` is visible. TextbookLens requires `finish_reason: "stop"` and `[DONE]` for
completion; `length` is treated as an incomplete answer. The same usage-only chunk rule applies.
([model list](https://platform.moonshot.ai/docs/api/list-models),
[chat API](https://platform.moonshot.ai/docs/api/chat),
[Kimi K3 quickstart](https://platform.moonshot.ai/docs/guide/kimi-k3-quickstart))
