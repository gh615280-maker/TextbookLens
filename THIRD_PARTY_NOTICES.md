# Third-party notices

This is an engineering inventory, not legal advice or a legal review. It is based on committed lockfiles and provenance records. A release artifact must be reviewed again when one exists.

## Reproducible review

| Scope                  | Inputs                                 | Command                                                 |
| ---------------------- | -------------------------------------- | ------------------------------------------------------- |
| npm production graph   | `package-lock.json`                    | `npm.cmd run check:licenses`                            |
| Rust Windows x64 graph | `src-tauri/Cargo.lock`, `deny.toml`    | `cargo deny --manifest-path src-tauri/Cargo.toml check` |
| Direct dependencies    | `package.json`, `src-tauri/Cargo.toml` | Lockfile review plus checks above                       |
| Fixtures/font          | `fixtures/README.md`, fixture sources  | `npm.cmd run fixtures:verify`                           |

The npm checker enforces the allowlist/exceptions in `scripts/check-npm-licenses.mjs`; its previous audit recorded 197 production packages. Rust policy uses `deny.toml` for `x86_64-pc-windows-msvc`, exact pins/lockfile, allowed registries, and named license/advisory exceptions. Counts and policies are review inputs, not a substitute for a distributor’s legal obligations.

## Dependency and resource classes

Direct runtime families include Tauri, React, React Aria, PDF.js, EPUB.js, Mammoth, KaTeX, DOMPurify, React Markdown/remark/rehype, SQLite/SQLx, reqwest, and Windows credential-store components. Direct development families include TypeScript, Vite, ESLint, Vitest, Playwright, fixture generators, and the npm license checker. Transitive dependencies are determined only by the lockfiles; this short list is not complete attribution.

Project-owned synthetic textbook, image, loopback-provider, and generated fixture material is Apache-2.0 as recorded in `fixtures/README.md`. `NotoSansSC-fixture-subset.otf` remains under SIL Open Font License 1.1; its complete unmodified text is committed at `fixtures/source/fonts/OFL.txt`. The fixture record includes upstream source/release and hashes. KaTeX and other package resources remain subject to their package-distributed licenses in the locked graph. No license text is copied selectively or incompletely here.

No installer or release bundle exists in this task. Task 7 must scan final allowlisted resources, notices, source maps (if any), and unpacked package. Signing, package, and publish checks are **NOT RUN**.
