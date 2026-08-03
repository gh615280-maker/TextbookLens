# ADR 0005: Capability-gated multimodal provider contract

## Status

Accepted for replanned V1 on 2026-08-03. The exact-model facts below were reverified from official
first-party documentation on 2026-08-03 and are stored with that `last_verified` date.

## Context

The existing `AiProvider` contract and embedded registry describe text chat only. Replanned V1 also needs bounded image learning and strict page analysis, but not every provider or model exposes the same image, PDF, schema, upload, streaming, or completion semantics. Treating all configured profiles as equivalent would cause unsupported requests, silent schema drift, or accidental image disclosure.

## Decision

### Capability truth is explicit and conservative

Each registered model declares independently:

- `text_chat`
- `image_input`
- `pdf_input`
- `strict_structured_output`
- context and output limits
- conservative image count, encoded bytes, decoded pixels, and dimension limits
- `last_verified`

Feature support is `supported`, `unsupported`, or `unknown`, not an optimistic Boolean default. Unknown/unregistered models may use the existing conservative text window only after provider validation; they are never considered image/PDF/schema capable without verified metadata.

Profile selection is operation-specific. The business layer asks the registry for a compatible learning or indexing profile and never branches on `ProviderKind`.

The checked-in registry is schema version 1. A supported image declaration is invalid without a
complete, positive, internally consistent `ImageLimits` record. Limits are application ceilings,
not vendor promises. Unknown fields, missing verification dates, duplicate models, default-model
mismatches, and invalid limits fail registry loading before the application opens a window.

### Exact-model capability record (verified 2026-08-03)

| Provider and exact model     | Image input | Native PDF/page-visual input | Strict schema | Official evidence and boundary                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                  |
| ---------------------------- | ----------- | ---------------------------- | ------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| OpenAI `gpt-5.6`             | Supported   | Supported                    | Supported     | The exact [GPT-5.6 Sol model record](https://developers.openai.com/api/docs/models/gpt-5.6-sol) declares image input, streaming, structured outputs, a 1,050,000-token context window and 128,000 maximum output. The official [vision guide](https://developers.openai.com/api/docs/guides/images-vision) uses `gpt-5.6` and documents image formats; [file inputs](https://developers.openai.com/api/docs/guides/file-inputs) document PDF text plus page images for vision-capable GPT-4o-and-later models. This is exact-model plus official protocol evidence, not a family-name inference.                                                                                |
| Google `gemini-3.6-flash`    | Supported   | Supported                    | Supported     | The exact [Gemini 3.6 Flash model record](https://ai.google.dev/gemini-api/docs/models/gemini-3.6-flash) lists image and PDF input, structured outputs, a 1,048,576-token input limit and 65,536-token output limit. The official [image](https://ai.google.dev/gemini-api/docs/image-understanding), [document](https://ai.google.dev/gemini-api/docs/document-processing), and [structured output](https://ai.google.dev/gemini-api/docs/structured-output) guides contain exact-model examples and protocol limits.                                                                                                                                                          |
| Anthropic `claude-sonnet-5`  | Supported   | Supported                    | Supported     | The exact [Claude Sonnet 5 record](https://platform.claude.com/docs/en/about-claude/models/whats-new-sonnet-5) gives the API ID, 1M context and 128K output. Anthropic's current active-model [vision](https://platform.claude.com/docs/en/build-with-claude/vision), [PDF](https://platform.claude.com/docs/en/build-with-claude/pdf-support), and [structured outputs](https://platform.claude.com/docs/en/build-with-claude/structured-outputs) contracts include Sonnet 5 through their stated active/current-model coverage.                                                                                                                                               |
| DeepSeek `deepseek-v4-flash` | Unsupported | Unsupported                  | Supported     | The exact official [chat completion schema](https://api-docs.deepseek.com/api/create-chat-completion) accepts string text content for this model and exposes no image or PDF content part. Direct `json_object` mode is not treated as strict schema. Strict schema support is limited to the official beta [strict tool-call mode](https://api-docs.deepseek.com/guides/tool_calls), which constrains declared parameters; therefore the model's schema flag is supported but visual and aggregate `StructuredPageAnalysis` operations are locally unsupported. Exact limits come from the official [current model facts](https://api-docs.deepseek.com/quick_start/pricing/). |
| Moonshot `kimi-k3`           | Supported   | Unsupported                  | Supported     | The exact [Kimi K3 quickstart](https://platform.kimi.ai/docs/guide/kimi-k3-quickstart) documents native vision, 1M context/output ceilings and strict `json_schema`; the [vision guide](https://platform.kimi.ai/docs/guide/use-kimi-vision-model) defines accepted image inputs. The official [file-QA flow](https://platform.kimi.ai/docs/guide/use-kimi-api-for-file-based-qa) extracts PDF text before placing it in a prompt, so it is not native PDF page-visual input and the registry does not promote it to PDF support.                                                                                                                                               |

Every supported image model currently uses the same narrower application ceiling: at most four
images, 4 MiB encoded bytes per image, 12 MiB encoded bytes total, 4,096 pixels on either axis,
and 8,847,360 decoded pixels per image. These values fit beneath all verified protocols and leave
headroom for JSON/base64 overhead under Gemini's 20 MB inline-request limit. They can be tightened
without changing provider wire semantics.

PDF support is recorded independently from image support. Phase 5 page analysis sends bounded page
images, so `StructuredPageAnalysis` requires explicit text, image, and strict-schema support; PDF
support alone neither enables nor disables that operation.

### Keep three narrow operations

The provider layer exposes separate operations rather than one large optional request:

```rust
enum AiOperation {
    TextLearning,
    VisionLearning,
    StructuredPageAnalysis,
}

trait AiProvider {
    async fn validate(...);
    async fn stream_text(...);
    async fn stream_vision(...);
    async fn analyze_pages(...);
}
```

An adapter may return `UNSUPPORTED_PROVIDER_CAPABILITY` locally before credential lookup or network access. Text adapters remain valid without implementing the two visual methods.

P5-A declares only these shared operations and local denial behavior. Provider request fields,
schema dialects, response parsing, and visual success terminals are deliberately deferred to P5-B.

### Binary content stays inside Rust-owned request boundaries

- UI capture is accepted only through a bounded IPC DTO tied to `book_id`, an app-issued capture ID, MIME allowlist, dimensions, byte length, and content hash.
- Raw image bytes become a Rust-only `VisionAsset` backed by zeroizing buffers or an app-owned temporary file. Generated TypeScript bindings expose metadata/capture handles, not provider wire payloads.
- Provider URLs and credentials remain fixed in Rust. No browser provider fetch and no user-configurable production origin is added.
- Adapter code owns base64/file upload fields, schema syntax, vendor hidden reasoning fields, event parsing, and terminal success rules.

The shared boundary admits only PNG, JPEG, and WebP after checking declared MIME against magic
bytes, parsing dimensions before allocation, enforcing encoded/decoded limits, and binding each
asset to its book and app-issued capture ID. Duplicate capture IDs or hashes within a request are
rejected. Raw bytes use zeroizing Rust buffers and have redacted `Debug`; they have no
`Serialize`, Tauri command DTO, or TypeScript binding.

### Structured results are untrusted

`analyze_pages` returns a bounded provider response to a versioned local decoder. The adapter does not directly insert blocks. Schema parsing, book/page ownership, batch membership, coordinate, count, string, ordering and source validation occur before a page transaction.

P5-A's decoder caps bytes before JSON parsing and uses `deny_unknown_fields` plus an exact schema
version. Its output remains explicitly `UntrustedPageAnalysis`; database ownership, coordinate,
ordering, semantic, and transaction validation belong to the later indexing phase.

### Remote temporary resources are optional and tracked

Inline image requests are preferred. If a verified official protocol requires remote files, the adapter returns an opaque cleanup handle. Handles are stored without content, never used as the sole index copy, and are retried on success, failure, cancellation, book deletion, and startup recovery.

Verified cleanup facts are protocol-specific: OpenAI supports inline data and explicit
[`DELETE /files/{id}`](https://developers.openai.com/api/reference/resources/files/methods/delete);
Gemini inline data avoids persistence, while its [Files API](https://ai.google.dev/gemini-api/docs/files)
documents automatic 48-hour expiry and manual deletion; Anthropic inline content avoids files,
while uploaded [Files](https://platform.claude.com/docs/en/build-with-claude/files) persist until
explicit deletion; DeepSeek has no visual upload path in the scoped exact-model chat protocol;
Kimi accepts inline base64 or an uploaded file ID and its file-QA guide demonstrates deletion.
P5-A exposes only a safe cleanup status. The provider and secret opaque ID remain Rust-only.

The existing Phase 4R terminal rules are not generalized: OpenAI requires its explicit completed
response event, Gemini requires its accepted finish reason, Anthropic requires the accepted stop
reason followed by `message_stop`, and DeepSeek/Kimi require the accepted finish reason followed by
`[DONE]`. EOF, HTTP success, usage, visible text, or `[DONE]` alone never becomes success. Visual
adapters must prove their exact terminal variants in P5-B before any capability is operational.

## Consequences

- UI capability labels are truthful and can show that one profile supports text but not visual indexing.
- Capability registry changes require generated-binding, official-source, security and cross-provider contract tests.
- Adding a model is metadata plus adapter evidence, not a provider-kind heuristic.
- Visual requests have more local staging work, but provider incompatibility is rejected before sending user content.

## Required evidence

- Official capability links and `last_verified` for every enabled model feature.
- Unknown-model denial tests for vision and structured analysis.
- Exact request/terminal/cancel fixtures for each implemented visual adapter.
- Tests proving image bytes, textbook text, schemas, vendor bodies, credentials and hidden reasoning never enter logs, DTO errors or diagnostics.
