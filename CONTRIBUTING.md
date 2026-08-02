# Contributing to TextbookLens

TextbookLens is a local-first Windows desktop application. Keep changes within
the documented product scope and preserve the Rust-owned database, credential,
and provider-network boundaries.

## Local setup

Install the pinned Node.js and Rust toolchains from the dependency baseline,
then run `npm ci` and `npx playwright install chromium`. Use the commands in
`docs/testing/verification-matrix.md` and the master implementation plan before
submitting a change.

## Change safety

- Never commit API keys, authorization headers, user textbooks, provider
  response bodies, or unrelated workspace files.
- Store credentials only through the Rust credential interface; frontend code
  uses provider profile IDs after save.
- Add files to Git with explicit paths. Do not use broad staging or destructive
  cleanup commands in a workspace containing user files.
- Add focused tests for behavior changes and keep generated TypeScript bindings
  current.
- Use only dependencies accepted by the npm and Cargo license policies.

## Required checks

Run the Phase 1 commands recorded in the verification matrix. At minimum, a
change must pass formatting, linting, type checking, frontend and Rust tests,
generated-binding checks, sensitive-file checks, license checks, and the
relevant build smoke test.

## Security reports

Do not open a public issue containing a credential, private textbook excerpt,
or exploitable security detail. Use the repository's private GitHub security
advisory channel and include only the minimum redacted reproduction metadata.
