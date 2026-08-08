# Contributing to TextbookLens

TextbookLens is a local-first Windows desktop application. Keep each change in formal V1 scope and preserve Rust-owned database, credential, provider, and network boundaries.

## Change safety

- Read applicable ADRs, the [V1 acceptance standard](docs/testing/2026-08-08-textbooklens-v1-executable-acceptance-standard.md), and the current verification matrix.
- Check `git status --short`; do not reformat, move, stage, delete, or commit unrelated workspace/user files.
- Stage explicit paths only. Never use `git add .`, `git add -A`, destructive cleanup, history rewriting, or amend to absorb unrelated work.
- Keep provider networking in Rust. Browser provider fetches and user-configurable production origins are prohibited.

## Data and secret safety

Never commit, log, fixture, screenshot, or test with a real API key, credential identifier, textbook, page image, source path, prompt, answer, note, teaching instruction, vendor body, or remote-resource ID. Use self-made synthetic inputs and run `npm.cmd run check:sensitive`. Credentials belong only in the Rust credential interface and Windows Credential Manager. Generated bindings and safe DTOs must not expose credential material or provider wire payloads. Preserve redaction in logs, errors, diagnostics, browser state, backups, and artifact scans.

## Database, generated bindings, and fixtures

- Migrations are append-only forward migrations. Never rewrite, renumber, move, or alter existing migration checksums; test fresh and historical-upgrade paths when adding one.
- Change Rust DTOs and generated TypeScript bindings together. Run `npm.cmd run check:generated`; do not hand-edit generated bindings as a substitute.
- Fixtures must be deterministic, synthetic, and license-provenanced. Record source/version/license/hash changes in `fixtures/README.md`, run `npm.cmd run fixtures:verify`, and never add copyrighted textbook content.
- Dependency changes require exact pin, official source, necessity, license, and sensitive-file review. Run `npm.cmd run check:licenses`; Cargo changes also require `deny.toml` review.

## Windows-isolated Rust work

Do not build into, relink, modify, or terminate a user-designated executable. For Rust checks use a unique, dedicated target/temp root outside the workspace with one job and no incremental state:

```powershell
$env:CARGO_TARGET_DIR = 'D:\CodexBuild\textbooklens-contrib-target'
$env:TEMP = 'D:\CodexBuild\textbooklens-contrib-temp'
$env:TMP = 'D:\CodexBuild\textbooklens-contrib-temp'
$env:CARGO_INCREMENTAL = '0'
$env:CARGO_BUILD_JOBS = '1'
cargo test --manifest-path src-tauri/Cargo.toml -j 1
```

Never point these variables at a user build, app-data directory, workspace root, or broad system location.

## Verification

For documentation/metadata work run:

```powershell
npm.cmd run format:check
npm.cmd run check:sensitive
npm.cmd run check:licenses
npm.cmd run fixtures:verify
git diff --check
```

Code, migration, generated-binding, or release changes also require their focused tests and acceptance-standard gates. Do not mark unexecuted installer, real provider, clean Windows, screen-reader, signing, package, or publishing gates as passed.

## Security reports

Follow [SECURITY.md](SECURITY.md). This checkout has no configured private reporting address; do not expose sensitive reproduction details publicly.
