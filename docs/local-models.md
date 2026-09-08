# Offline local models

In **Settings → AI services**, select **Auto-connect local AI**. The same button is available during onboarding. TextbookLens discovers an existing Ollama or LM Studio installation, starts its local service when needed, finds downloaded text models, and tests a model with a short synthetic question. It adds the discovered models and selects a tested model as the default for text answers. Repeating the action updates existing profiles instead of duplicating them.

No API key is needed for the supported unauthenticated local services. The application does not install software, download models, use cloud models, or fall back to a cloud provider when a local request fails. A service that requires authentication is reported separately; TextbookLens does not change its authentication settings.

## Supported workflow

- **Ollama:** discover its executable from the normal Windows installation or PATH; use the local `OLLAMA_HOST` port when configured, otherwise 11434. Start `ollama serve` if the service is stopped. Use `/api/tags` and `/api/show` to find installed completion models. Reject cloud/remote models before each inference. Load the model on demand through `/api/chat`.
- **LM Studio (experimental; real-machine validation deferred to v0.2.1):** discover `lms.exe` from the LM Studio home or PATH and its saved local server port (default 1234). Start the server with the installed CLI. Read `lms ls --llm --json`, exclude models on other devices, and use `lms load` when needed. LM Link-aware versions are loaded with `--local`, and the resulting local instance is verified before sending a question.
- Models are stored as separate profiles, with their own model identifier, context budget, and numeric loopback port. Use **Use for learning** to switch models. Local profiles do not create or require Windows credentials.
- Local models support text selection questions, book questions, follow-up answers, and the teaching-instruction test. Ollama models whose installed metadata advertises vision can also be selected with **Use for vision** for image-region questions. Image input is checked again before every request; text-only and remote models never receive images. LM Studio vision, structured page indexing, and cloud file extraction remain unavailable for local profiles in this version.

Vision capability is saved per local profile and survives restart. One request accepts one PNG/JPEG/WebP image, at most 2 MiB encoded, 2048 pixels on either side, and 1,048,576 decoded pixels. TextbookLens reserves additional context for that image. Discovered Ollama vision models use up to 16,384 context tokens, subject to the model's declared limit. Reconnecting retains an existing connected local text default; select a vision default independently in AI services.

The software must be functional and its inference engine and model files must already be installed. Sufficient RAM/VRAM is still required. Initial model loading can take several minutes. A missing runtime, missing text model, authentication requirement, or failed answer test is shown in the connection result.

All TextbookLens local HTTP requests use `127.0.0.1`, bypass proxies, and refuse redirects. Application-level validation checks model locality as well: an Ollama localhost endpoint can serve cloud models, and LM Studio can expose remote devices through LM Link.

## References

- [Ollama model list](https://docs.ollama.com/api/tags), [cloud/local model distinction](https://docs.ollama.com/cloud).
- [LM Studio CLI model list](https://lmstudio.ai/docs/cli/local-models/ls), [model loading](https://lmstudio.ai/docs/cli/local-models/load), [server startup](https://lmstudio.ai/docs/cli/serve/server-start).
- [LM Studio CLI implementation](https://github.com/lmstudio-ai/lms): `src/subcommands/list.ts` and `src/subcommands/load.ts` define local device metadata and local-only loading.

## Verification

Synthetic loopback tests cover local/remote filtering, credential-free persistence and deletion, repeated discovery, stream termination, and rejection of redirects. Learning preparation tests cover both selection and book questions without credential access. The database migration contract covers existing profile references and historical upgrades.

An opt-in Rust test, `ai::local::tests::installed_local_runtime_offline_smoke`, exercises discovery, persistence, the normal provider runtime, and a real streamed answer using installed software and a temporary database. It never uses a user's books or cloud keys. Run it with `cargo test --manifest-path src-tauri/Cargo.toml --lib installed_local_runtime_offline_smoke -- --ignored`, using the isolated build directories described in CONTRIBUTING.md.

Verified on 2026-09-07: 55 frontend tests, 347 Rust unit tests, and 37 binding, database, migration, and profile lifecycle tests passed. The installed Ollama 0.33.3 service with `deepseek-r1:8b` passed discovery, registration, and a complete streamed answer. A separate unused loopback port verified automatic startup and discovery of already downloaded models; that test-owned service was stopped afterward. LM Studio model filtering and stream handling have automated coverage; an actual LM Studio installation was not available for an end-to-end run on this machine.
