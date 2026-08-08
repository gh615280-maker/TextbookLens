# License and dependency review record

TextbookLens declares `Apache-2.0` in `package.json` and `src-tauri/Cargo.toml`. This file is not a complete copy of license text, legal notice, or legal opinion.

## npm policy

`npm.cmd run check:licenses` requires exact direct semver declarations, lockfile version 3, npm HTTPS registry sources, SHA-512 integrity values, and licenses within this policy: Apache-2.0, MIT, BSD-2-Clause, BSD-3-Clause, ISC, Unicode-3.0, and Zlib. Its version-specific exceptions are `tslib@2.8.1` 0BSD and normalization of `duck@0.1.12` from `BSD*` to BSD-2-Clause.

## Rust policy

`cargo deny --manifest-path src-tauri/Cargo.toml check` applies `deny.toml` to the locked Windows x64 graph. Its base list matches the npm policy and has named MPL-2.0 exceptions for `cssparser`, `cssparser-macros`, `dtoa-short`, `option-ext`, and `selectors`. It denies unknown registries/git sources and records current advisory exceptions with reasons. Do not broaden a policy without dependency, source, license, and security review.

## Fixture provenance

See [fixture provenance and license records](fixtures/README.md) and the full font license at `fixtures/source/fonts/OFL.txt`. Fixture material must be self-made/synthetic or have documented license provenance and hashes.

## Verification and limitation

```powershell
npm.cmd run check:licenses
npm.cmd run fixtures:verify
cargo deny --manifest-path src-tauri/Cargo.toml check
```

These commands are a reproducible technical review of resolved graphs and fixtures. They do not provide legal advice, identify every downstream attribution duty, or certify a release artifact. See [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) for scope and release limitation.
