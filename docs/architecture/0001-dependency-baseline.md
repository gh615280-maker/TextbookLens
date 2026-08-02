# 0001: Reproducible dependency baseline

## Status

Accepted on 2026-08-02.

## Decision

TextbookLens uses committed npm and Cargo lockfiles. The project is a Windows
x64 desktop application built with Tauri 2, React, TypeScript, Vite, Rust, and
SQLite. The frontend never opens the application database.

## Resolved toolchain

- Node.js 24.18.1; npm 11.16.0
- Rust 1.97.1 (`x86_64-pc-windows-msvc`); Cargo 1.97.1
- Visual Studio Build Tools 2022 17.14.37 with `VC.Tools.x86.x64`

## Top-level packages

- Frontend: Tauri API 2.11.1, dialog plugin 2.7.2, React/React DOM 19.2.8,
  React Router 7.18.2, React Aria Components 1.20.0, Zod 4.4.3.
- Tooling: Tauri CLI 2.11.4, TypeScript 7.0.2, Vite 8.2.0, Vitest 4.1.10,
  ESLint 10.8.0, Prettier 3.9.6, Playwright 1.62.1,
  license-checker-rseidelsohn 5.0.1, and cargo-deny 0.20.2.
- Rust: Tauri 2.11.5, SQLx 0.9.0, ts-rs 12.0.1, keyring 4.1.6, Tokio 1.53.1,
  Serde 1.0.229, UUID 1.24.0, and the exact direct dependencies in
  `src-tauri/Cargo.toml`.

`eslint-plugin-jsx-a11y` 6.10.2 declares ESLint 3–9 compatibility while the
approved baseline pins ESLint 10.8.0. `.npmrc` sets `legacy-peer-deps=true` so
both approved exact versions install reproducibly until the pinned plugin is
updated by a later approved dependency-baseline change.

## License policy

The Phase 1 allowlist is Apache-2.0, MIT, BSD-2-Clause, BSD-3-Clause, ISC,
Unicode-3.0, and Zlib. Phase 1 CI enforces it for the full resolved graphs.

One package-level exception is recorded for `tslib@2.8.1`, a locked production
transitive dependency of React Aria Components. Its SPDX license is 0BSD, a
permissive license with no attribution or source-disclosure obligation. The npm
license check accepts 0BSD only for that exact package and version; 0BSD is not
added to the general allowlist.

Phase 2 adds two dependencies with SPDX choice expressions: DOMPurify 3.4.12 is
`MPL-2.0 OR Apache-2.0`, and JSZip 3.10.1 is
`MIT OR GPL-3.0-or-later`. TextbookLens selects the already-allowed Apache-2.0
and MIT branches respectively; the license checker evaluates `OR` as a choice
and does not add MPL or GPL to the general allowlist. The locked transitive
`duck@0.1.12` declares the non-standard value `BSD`, which the metadata tool
reports as `BSD*`; its bundled `LICENSE` is the two-clause BSD text. The checker
normalizes that value to `BSD-2-Clause` only for this exact package and version.

Tauri's locked Rust graph contains five MPL-2.0 crates: `cssparser@0.36.0`,
`cssparser-macros@0.6.1`, `dtoa-short@0.3.5`, `option-ext@0.2.0`, and
`selectors@0.36.1`. They are package/version-specific cargo-deny exceptions,
not general MPL approval. TextbookLens does not modify their covered source
files. Distribution must preserve their notices, identify MPL-licensed files,
and make the exact covered source available; any future modification to those
files remains under MPL-2.0. Phase 7 release notices own this obligation.

Cargo-deny evaluates the supported `x86_64-pc-windows-msvc` target and denies
all unmaintained advisories except five informational rust-unic 0.9 advisories:
`RUSTSEC-2025-0075`, `RUSTSEC-2025-0080`, `RUSTSEC-2025-0081`,
`RUSTSEC-2025-0098`, and `RUSTSEC-2025-0100`. These crates are fixed transitives
of `tauri-utils@2.9.3` through `urlpattern@0.3.0`; the advisories state that no
safe upgrade is available. They are not vulnerability advisories. Re-evaluate
and remove the exceptions with the next Tauri dependency-baseline upgrade.

`npm audit` reports `GHSA-qwww-vcr4-c8h2` against the pinned React Router
7.18.2. The upstream advisory states that it affects only unstable React Server
Components APIs. TextbookLens uses React Router in client-only library mode,
has no server or server actions, and does not import an RSC API, so the affected
execution path is absent. Keep the planned version for Phase 1; re-evaluate this
decision before any server or RSC scope is proposed, or when a compatible v7
patch is published.

## Primary sources

- [Node.js releases](https://nodejs.org/en/about/previous-releases)
- [Rust installation](https://www.rust-lang.org/tools/install)
- [Tauri documentation](https://v2.tauri.app/)
- [npm registry](https://www.npmjs.com/)
- [crates.io](https://crates.io/)
