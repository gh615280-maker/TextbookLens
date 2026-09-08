# AI services

TextbookLens supports user-configured official OpenAI, Gemini, Anthropic, DeepSeek, and Kimi services, plus [Ollama local text/image models and experimental LM Studio text models](local-models.md). Requests leave from the Rust desktop layer; the browser UI does not fetch providers or configure a production base URL. Adding or replacing a cloud key validates it before a key/profile update is committed. Cloud keys are in Windows Credential Manager, not profile lists, SQLite/frontend state, or backups. Local profiles use a discovered loopback port and require no cloud key.

## Configured capability registry

The embedded registry below is an implementation boundary, not a price, availability, quota, privacy, or future-model promise. “Page operation” requires explicit text, image, and strict-schema support; native PDF input alone does not enable it. Application image limits are at most 4 images, 4 MiB each, 12 MiB total, 4096 pixels on either side, and 8,847,360 decoded pixels per image.

| Provider  | Exact default model | Text      | Image       | Native PDF  | Strict structured output   | Page operation | Registry verification |
| --------- | ------------------- | --------- | ----------- | ----------- | -------------------------- | -------------- | --------------------- |
| OpenAI    | `gpt-5.6`           | Supported | Supported   | Supported   | Supported                  | Supported      | 2026-08-03            |
| Gemini    | `gemini-3.6-flash`  | Supported | Supported   | Supported   | Supported                  | Supported      | 2026-08-03            |
| Anthropic | `claude-sonnet-5`   | Supported | Supported   | Supported   | Supported                  | Supported      | 2026-08-03            |
| DeepSeek  | `deepseek-v4-flash` | Supported | Unsupported | Unsupported | Supported for strict tools | Denied locally | 2026-08-03            |
| Kimi      | `kimi-k3`           | Supported | Supported   | Unsupported | Supported                  | Supported      | 2026-08-03            |

An unregistered/unknown model receives only conservative verified text behavior after validation. Image, PDF, and strict-schema capability is fail-closed: the app rejects the operation before credential lookup or network access. Do not infer support from a provider name or model family.

## Official references

These pages were checked as reachable on **2026-08-08**. They support the registry evidence and do not replace the app’s explicit capability gate.

| Provider  | Official reference                                                                                                                                                  | Verification                                |
| --------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------- |
| OpenAI    | [GPT-5.6 model](https://developers.openai.com/api/docs/models/gpt-5.6-sol), [vision](https://developers.openai.com/api/docs/guides/images-vision)                   | Reachable, official `developers.openai.com` |
| Gemini    | [Gemini 3.6 Flash](https://ai.google.dev/gemini-api/docs/models/gemini-3.6-flash)                                                                                   | Reachable, official `ai.google.dev`         |
| Anthropic | [vision documentation](https://platform.claude.com/docs/en/build-with-claude/vision)                                                                                | Reachable, official `platform.claude.com`   |
| DeepSeek  | [chat-completion documentation](https://api-docs.deepseek.com/api/create-chat-completion)                                                                           | Reachable, official `api-docs.deepseek.com` |
| Kimi      | [Kimi K3 quickstart](https://platform.kimi.ai/docs/guide/kimi-k3-quickstart), [Files-based Q&A](https://platform.kimi.ai/docs/guide/use-kimi-api-for-file-based-qa) | Reachable, official `platform.kimi.ai`      |

See [ADR 0004](architecture/0004-provider-protocols.md) and [ADR 0005](architecture/0005-multimodal-provider-capabilities.md) for protocol evidence. Recheck official sources and `last_verified` whenever the registry changes.

## Sending and confirmation

Every AI invocation is a new, user-started request. Learning sends a frozen selection, needed same-book context/history, current instruction, and question. Visual learning adds only an explicitly confirmed bounded region/page image. Optional PDF indexing sends confirmed exceptional pages only. Kimi full-text preparation sends an explicitly selected PDF only after region binding. Book questions use a current-book table of contents, relevant local retrieval, instruction, question, and same-book history—not the entire book.

The first region/page-image send, AI-assisted indexing start, and a detected cost-risk action require confirmation naming provider, model, content category, image/page count, and risk. Declining sends nothing. A saved choice is per profile/category and cannot permit background work. Hiding a panel does not cancel it; the explicit **Stop** action cancels only that request.

For Kimi, a no-content `GET /models` probe detects `cn` or `international` after a key is entered. There is no manual region selector. Only explicit authentication/region mismatch allows a second-region probe; timeout, rate-limit, network, and 5xx errors retain the original error. Chat, Files upload/polling/download, and cleanup use the bound region. Replacing a key repeats detection; cleanup uses the region where the resource was created.

Five real-provider credential gates remain **NOT RUN**. The current evidence is synthetic/loopback contract coverage, not a claim of current account, quota, model availability, or real request success.
