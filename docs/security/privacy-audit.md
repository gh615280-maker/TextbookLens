# TextbookLens privacy, leakage, and supply-chain audit

Status date: 2026-08-08. This is the Phase 15 Task 3 audit of the V1 product
checkpoint. It is not a release approval. Clean Windows installation, signed
installer contents, uninstall, five real-provider runs, and publication remain
unexecuted release gates.

## Conclusion

The repository-controlled source, fixtures, generated outputs, frontend
distribution, Tauri resources, migrations, dependency manifests, and lockfiles
pass the maintained sensitive-value matrix. The production network boundary is
Rust-only, uses rustls, disables proxies, redirects, referrers, and automatic
retries, and contains only the six specified provider origins. npm and Cargo
direct dependencies are exact-pinned and their locked sources are constrained.

No installer or release bundle exists. The authoritative V1 debug executable is
43,559,424 bytes with SHA-256
`38F6652F6F4B60185046DAB8426A3E9C0B69E912BE75118CA68B19D83F290521`.
Its two-link Cargo identity is rejected before scanning. A detached single-link,
byte-identical snapshot was then scanned and rejected because debug information
contains a private absolute build path. The source hash was unchanged before and
after the read-only audit, and the temporary snapshot was removed. That finding
is not allowlisted and the debug executable is not a release artifact. Task 7
must build the final release/installer and rescan every packaged file; until then
there is deliberately no release-artifact PASS.

## Threat surface

The audit treats these as leakage or substitution boundaries:

- Git-controlled source, tests, fixtures, generated bindings, migrations, and
  documentation, including case, Unicode, escaped, encoded, and split values.
- Minified frontend bundles, source maps, Tauri resources, executable string
  data, installers, archives, database exports, logs, diagnostics, crash/error
  strings, and backup/restore material.
- IPC error DTOs, Rust `Debug` implementations, provider errors, browser
  storage, streaming state, and persisted history.
- Provider endpoint selection, Kimi region discovery/binding, frontend network
  APIs, redirects, proxies, cookie state, TLS selection, telemetry, and update
  channels.
- npm and Cargo direct pins, lockfile consistency, registries, integrity,
  licenses, advisories, feature activation, and fixture provenance.

The scanner never reads the real application-data directory, user documents,
the user's general temporary directory, credential storage, or arbitrary home
content. All dynamic privacy tests use `TempDir`, synthetic values, in-memory
credential stores, deterministic provider doubles, or loopback-only browser
traffic.

## Maintained sentinel matrix

Sentinels are generated at runtime. No credential, textbook excerpt, user note,
provider body, or private path is committed as fixture data.

| Class                     | Prohibited surfaces                                                        |
| ------------------------- | -------------------------------------------------------------------------- |
| API credential            | source, fixtures, artifacts, logs, diagnostics, IPC, backup, browser state |
| Credential key/identifier | source, artifacts, diagnostics, backup, browser state                      |
| Source absolute path      | artifacts, diagnostics, backup, IPC, browser state                         |
| Textbook body             | logs, diagnostics, error DTOs, unrelated artifacts                         |
| Page image/base64         | logs, diagnostics, error DTOs, browser persistence, unrelated artifacts    |
| Prompt                    | logs, diagnostics, error DTOs, unrelated artifacts                         |
| Teaching instruction      | logs, diagnostics, error DTOs, unrelated artifacts                         |
| Answer                    | logs, diagnostics, error DTOs, unrelated artifacts                         |
| Vendor response/body      | source, fixtures except synthetic protocol shapes, logs, diagnostics, IPC  |
| Remote resource ID        | logs, diagnostics, backup, browser state, unrelated artifacts              |
| User note                 | logs, diagnostics, error DTOs, unrelated artifacts                         |
| Internal request ID       | logs, diagnostics, unrelated artifacts                                     |
| Internal profile ID       | logs, diagnostics, unrelated artifacts                                     |
| Internal run ID           | logs, diagnostics, unrelated artifacts                                     |
| Internal attempt ID       | logs, diagnostics, unrelated artifacts                                     |

An answer, annotation summary, note, selection, or teaching instruction is not
globally forbidden. It is valid user data in its explicit local stream/history,
annotation, settings, database, or backup context. The audit instead proves it
does not cross into logs, errors, diagnostics, unrelated artifacts, or another
book. This avoids the invalid claim that a useful application contains zero
content.

## Scanner behavior and boundaries

`check-sensitive-files.mjs` enumerates only Git tracked, staged, and non-ignored
untracked paths under the repository root. It rejects protected-path
reintroduction, links, root escape, and files that change while open. It uses
overlapping 64 KiB reads and scans UTF-8, Latin-1/string-table data, UTF-16LE,
and UTF-16BE.

Each candidate is NFKC-normalized, case-folded, stripped of formatting
characters, and checked in compact form. The decoder recursively checks JSON
and JavaScript escapes, URL percent encoding, hexadecimal, base64, and URL-safe
base64 within bounded candidate and byte budgets. ZIP entry names and contents,
nested ZIPs, and gzip payloads are scanned. ZIP output is consumed as a bounded
stream rather than decompressed before limits are checked.

The default per-file limit is 256 MiB. Archive limits are 4,096 entries, depth
3, 64 MiB per expanded entry, and 512 MiB total expanded bytes. The artifact
auditor additionally limits a run to 12,000 files, depth 24, and 2 GiB total.
Limit exhaustion and unreadable archives are findings, never implicit PASS.

There is no sensitive-value allowlist. Ordinary words, placeholder Key fields,
local non-secret profile labels, and fixture-relative paths are negative test
cases, not broad exceptions. Generic credential shapes require a provider key
shape or a sufficiently long mixed token, preventing binary string-table field
names from being misclassified as secrets. The fixed protected-path deny list is
retained and cannot hide values.

`audit-release-artifacts.mjs` accepts only explicit absolute file/root targets.
It rejects relative paths, dot segments, UNC/device forms, environment syntax,
globs, filesystem/workspace/home or other broad roots, Windows device names,
links/junctions/reparse traversal, hardlinks, duplicate identities, type drift,
and read/enumeration races. It opens files read-only, never executes them, and
contains no network operation.

Default controlled scope and latest result:

| Scope                                                       | Result                                                             |
| ----------------------------------------------------------- | ------------------------------------------------------------------ |
| Frontend `dist`, including emitted source maps when present | SCANNED                                                            |
| Synthetic fixture corpus and nested archives                | SCANNED                                                            |
| Tauri resources/provider capability data                    | SCANNED                                                            |
| Immutable migrations 0001 through 0015                      | SCANNED                                                            |
| npm/Cargo manifests and lockfiles plus Cargo policy         | SCANNED                                                            |
| Authoritative debug executable                              | REJECTED: hard-linked Cargo identity                               |
| Verified single-link snapshot of the same debug bytes       | REJECTED: non-release debug metadata contains a private build path |
| Installer/release bundle                                    | NOT PRESENT; Task 7 rescan required                                |

The default controlled run scanned 142 files and 7,341,294 bytes with no
sentinel or generic credential finding. It did not scan any real user directory.

## Local data and data sent to providers

| Data                                         | Local behavior                                        | Remote behavior                                                                                                           |
| -------------------------------------------- | ----------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------- |
| Imported PDF/EPUB/DOCX                       | App-owned copy plus derived/index data                | Never sent merely by import                                                                                               |
| Local extracted text, FTS, locators, markers | SQLite/app-owned storage, book-isolated               | Only bounded selected/retrieved context after an explicit AI action                                                       |
| Selection/question/history                   | Stored only after the completion transaction          | Sent to the selected provider when the user starts that request                                                           |
| Page image/region capture                    | Ephemeral bounded staging; released on cancel/failure | Sent only after explicit vision authorization                                                                             |
| Teaching instruction                         | Local revisioned preference                           | Included only in a newly prepared authorized request                                                                      |
| Notes and annotation summaries               | Local durable user data; included in backup           | Sent only when explicitly selected as request context                                                                     |
| Provider credential                          | OS credential vault only                              | Authentication header to the selected official provider origin only                                                       |
| Kimi source extraction                       | Local source remains authoritative                    | `/models` region probe occurs before upload; an authorized source upload stays bound to its creation region until cleanup |
| Browser state                                | Panel geometry and reader viewport preferences only   | No production browser network path                                                                                        |

The Kimi Key format is never used to guess region. Timeout, rate-limit, and
server-failure results do not bind a region. Chat, Files, and remote cleanup keep
the creation region, whose database value is restricted to `cn` or
`international`.

## Backup, restore, SQLite, diagnostics, and browser state

A backup is private user data. It intentionally contains app-owned textbook
copies, the sanitized SQLite database, required derived files, reading state,
completed questions and answers, annotations, user notes, annotation summaries,
teaching preferences, and local index/correction/search state. It does not
contain API Keys, credential identifiers, provider remote-resource rows,
absolute source paths, logs, caches, partial images, transient prompts, vendor
bodies, or incomplete answers. Restore into an empty credential store requires
AI reconfiguration.

Synthetic integration tests inspect the real backup bytes, manifest hashes,
closed/restored SQLite state, and restore result. They distinguish expected
durable answers/summaries from prohibited runtime sentinels. Logs and caches are
seeded with synthetic prohibited values and proved absent from the archive.

`AppError` diagnostics now redact all matrix field names plus private home paths
and TextbookLens credential identifiers before tracing. Provider failures expose
stable categories rather than response bodies. Request/vision/structured result
and remote-cleanup `Debug` implementations remain structural. IPC error DTOs
contain a stable code, localized message/next step, and optional diagnostic ID,
not diagnostic detail.

Production browser storage contains only layout/viewport preferences. There is
no production `fetch`, XHR, WebSocket, EventSource, cookie, IndexedDB, credential,
answer, note, or provider-body storage path in frontend code.

## Network and provider boundary

The exact production origin allowlist is:

- `https://api.openai.com/`
- `https://generativelanguage.googleapis.com/`
- `https://api.anthropic.com/`
- `https://api.deepseek.com/`
- `https://api.moonshot.cn/v1/`
- `https://api.moonshot.ai/v1/`

No custom production host is accepted. Both normal provider transport and Kimi
Files use `no_proxy`, rustls, no redirects, no referrer, and no automatic retry.
The Cargo feature graph enables reqwest rustls, JSON, stream, multipart, gzip,
and brotli. It does not enable reqwest cookies, SOCKS, proxy support, native TLS,
or a cookie store. Multipart is owned by the explicit Kimi Files upload path.
No telemetry, analytics, Sentry transport, browser production fetch, or
unspecified update channel was found.

## Dependencies, licenses, and provenance

All npm runtime and development direct specifications are exact semver values.
The root lock record must match the manifest, each direct locked version must
match its declaration, every non-root npm package must resolve from the npm HTTPS
registry, and every record must have SHA-512 integrity. The production license
policy passes 197 packages.

The npm license allowlist remains limited to Apache-2.0, MIT, BSD-2-Clause,
BSD-3-Clause, ISC, Unicode-3.0, and Zlib. The only npm exceptions remain exact
package/version evidence: one 0BSD package and one version-scoped `BSD*` metadata
normalization backed by its bundled two-clause text. No dependency-wide or
value-wide exception was added.

Every direct Cargo dependency, including build, development, and Windows target
dependencies, uses an exact `=version` registry pin. Target-filtered `cargo
metadata --locked --offline` proves the Windows lock graph is consistent without
fetching uncached non-Windows crates. Cargo deny passes advisories, bans,
licenses, and sources with the existing exact MPL file-level exceptions and five
recorded unmaintained transitive Unicode advisories owned by the pinned Tauri
graph; no git source or unknown registry is allowed.

`npm.cmd ls --omit=dev --all --json` succeeds for the production graph and finds
no proxy, SOCKS, cookie, telemetry, or analytics package. The full development
tree reports an existing peer-range mismatch: the pinned accessibility ESLint
plugin declares support only through ESLint 9 while the verified lint toolchain
uses ESLint 10. The mismatch is development-only; lint passes, and Task 3 does
not perform a forbidden major downgrade/upgrade. Proxy, SOCKS, cookie, and
OpenTelemetry packages found in the full tree belong only to Playwright/jsdom or
other development tooling, not production.

The offline npm advisory cache reports zero known vulnerabilities for the
current lock (0 at every severity). No `npm audit fix`, package installation,
major upgrade, or online advisory refresh was performed.

All textbook/vision/provider fixtures are project-owned synthetic material.
Generated PDF/EPUB/DOCX files derive deterministically from the committed
synthetic source. Tiny image fixtures are self-made. The only third-party fixture
asset is a recorded font subset retaining its full SIL Open Font License notice,
upstream release identity, and source/subset hashes. `fixtures:verify` is the
deterministic provenance gate.

## Accepted metadata

The following non-secret values are expected only in their documented contexts:
provider kind and model, Kimi region enum, capability flags, local status/counters
and timestamps, content hashes, migration/license identifiers, and a generated
diagnostic ID. Local object UUIDs may exist in SQLite and backups where required
for relational durability, but are not accepted as log/artifact leakage.
Filename/title metadata, user-visible answers, notes, summaries, and teaching
preferences are accepted local/backup data, not diagnostic or unrelated build
data. Debug-only compiler paths are not accepted for release and caused the
debug executable rejection above.

## Verification commands

The audit uses the fixed single-job Cargo target and task-specific temporary
directory. Private absolute values are supplied through environment variables
and are intentionally not copied into this document.

```powershell
npm.cmd test -- scripts/check-sensitive-files.test.mjs scripts/check-npm-licenses.test.mjs scripts/audit-release-artifacts.test.mjs
npm.cmd run check:sensitive
npm.cmd run check:licenses
node scripts/audit-release-artifacts.mjs
Get-FileHash -Algorithm SHA256 -LiteralPath $AUTHORITATIVE_DEBUG_EXE
node scripts/audit-release-artifacts.mjs --no-defaults --file $CONTROLLED_DEBUG_SNAPSHOT
npm.cmd run check:generated
npm.cmd run fixtures:verify
npm.cmd test
npm.cmd run format:check
npm.cmd run lint
npm.cmd run typecheck
npm.cmd audit --offline --json
npm.cmd ls --omit=dev --all --json
npm.cmd ls --all --json
cargo deny --manifest-path src-tauri/Cargo.toml --offline --locked check
cargo metadata --manifest-path src-tauri/Cargo.toml --format-version 1 --locked --offline --filter-platform x86_64-pc-windows-msvc
cargo tree --manifest-path src-tauri/Cargo.toml --locked --offline -e features -i reqwest
cargo test --manifest-path src-tauri/Cargo.toml --test redaction -j1
cargo test --manifest-path src-tauri/Cargo.toml --test local_data_privacy_checkpoint -j1
cargo test --manifest-path src-tauri/Cargo.toml --lib maintenance -j1
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --all-features -j1 -- -D warnings
git diff --check
git diff --cached --check
```

Focused scanner tests cover all positive/negative classes, compatibility case
and Unicode, escaped/URL/base64/hex forms, buffer splits, binary strings,
UTF-8/UTF-16, bundles/source maps, nested archive names/content, bounded archive
expansion, path safety, hardlinks/reparse points, duplicates, races, and stable
exit/error categories.

## Remaining mandatory Task 7 gate

Task 7 must create the release build and installer without using real user data,
then run this auditor over the final executable, resources, migrations, provider
registry, license notices, installer, unpacked bundle, source maps, and any
generated diagnostic/crash artifacts. It must fail on any private compiler path,
sentinel, credential shape, unapproved origin, missing notice, link/reparse,
duplicate identity, archive limit, or mutation race. Only that exact final
package may receive a release-artifact PASS.
