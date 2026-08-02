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
  ESLint 10.8.0, Prettier 3.9.6, Playwright 1.62.1.
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

## Primary sources

- [Node.js releases](https://nodejs.org/en/about/previous-releases)
- [Rust installation](https://www.rust-lang.org/tools/install)
- [Tauri documentation](https://v2.tauri.app/)
- [npm registry](https://www.npmjs.com/)
- [crates.io](https://crates.io/)
