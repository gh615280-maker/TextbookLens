# ADR 0004: Fixed first-party provider protocol matrix

- Status: Accepted for Phase 4 Task 1
- Last verified: 2026-08-02
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

| Provider | Production host | Validation operation | Generation operation | Required headers / request mapping | Visible delta / usage / completion | Ignore safely | Official sources |
| --- | --- | --- | --- | --- | --- | --- | --- |
| OpenAI | `https://api.openai.com` | `GET /v1/models/{model}` | `POST /v1/responses` with `stream: true` | `Authorization: Bearer <key>`, `Content-Type: application/json`; map model and normalized input to `model` and `input`. | `response.output_text.delta` / final response `usage` / `response.completed`. | reasoning, tool, audio, image, and unknown extension events. | [models](https://developers.openai.com/api/docs/models/gpt-5.6-sol), [Responses streaming](https://developers.openai.com/api/docs/guides/streaming-responses), [Responses API](https://platform.openai.com/docs/api-reference/responses) |
| Gemini | `https://generativelanguage.googleapis.com` | `GET /v1beta/models/{model}` | `POST /v1beta/models/{model}:streamGenerateContent?alt=sse` | `x-goog-api-key: <key>`, `Content-Type: application/json`; map normalized messages to `contents`, model output limit to `generationConfig.maxOutputTokens`. | `candidates[].content.parts[].text` / `usageMetadata` / a supported `candidates[].finishReason`. | non-text parts, thought parts, safety/grounding metadata, and unknown fields. | [model metadata API](https://ai.google.dev/api/models), [Gemini 3.6 Flash](https://ai.google.dev/gemini-api/docs/models/gemini-3.6-flash), [text generation](https://ai.google.dev/gemini-api/docs/generate-content/text-generation) |
| Anthropic | `https://api.anthropic.com` | `GET /v1/models/{model}` | `POST /v1/messages` with `stream: true` | `x-api-key: <key>`, `anthropic-version: 2023-06-01`, `content-type: application/json`; map system text to `system`, messages to `messages`, and output limit to `max_tokens`. | `content_block_delta` with `text_delta` / `message_delta.usage` / `message_stop`. | `ping`, tool/input JSON, thinking, signature, and unknown event types. | [models](https://platform.claude.com/docs/en/about-claude/models/overview), [Messages](https://platform.claude.com/docs/en/api/messages/create), [streaming](https://platform.claude.com/docs/en/build-with-claude/streaming), [versioning](https://platform.claude.com/docs/en/api/versioning) |
| DeepSeek | `https://api.deepseek.com` | `GET /models`, then require the selected ID in the returned model list | `POST /chat/completions` with `stream: true`, using the OpenAI-format API | `Authorization: Bearer <key>`, `Content-Type: application/json`; map normalized messages to `messages`, output limit to `max_tokens`, and model to `model`. | `choices[].delta.content` / final `usage` stream chunk / `choices[].finish_reason`. | `reasoning_content`, tool calls, logprobs, and unknown fields. | [API quick start](https://api-docs.deepseek.com/), [chat completions](https://api-docs.deepseek.com/api/create-chat-completion/), [current model facts](https://api-docs.deepseek.com/quick_start/pricing/) |
| Kimi | `https://api.moonshot.ai/v1` | `GET /models`, then require the selected ID in the returned model list | `POST /chat/completions` with `stream: true` | `Authorization: Bearer <key>`, `Content-Type: application/json`; map normalized messages to `messages`, output limit to `max_completion_tokens`, and model to `model`. | `choices[].delta.content` / final `usage` stream chunk / `choices[].finish_reason`. | `reasoning_content`, tool calls, and unknown fields. | [model list](https://platform.kimi.ai/docs/models), [Kimi K3](https://platform.kimi.ai/docs/guide/kimi-k3-quickstart), [chat API](https://platform.kimi.ai/docs/api/chat) |

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
