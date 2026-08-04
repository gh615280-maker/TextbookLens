# TextbookLens import fixtures

These tiny fixtures are generated deterministically from one shared semantic source for PDF, EPUB, and DOCX import tests. The metadata baseline is `2026-08-01T00:00:00.000Z`.

`scanned-textbook.pdf` and `mixed-quality-textbook.pdf` are self-made synthetic fixtures: their image-only pages are generated from deterministic colored rectangles, contain no source textbook content, and are licensed under Apache-2.0.

The `source/vision/` images are project-owned 2×2-pixel synthetic color swatches used only by loopback provider contract tests. They contain no textbook or user content and are licensed under Apache-2.0.

## Licensing

- `textbook-content.json`, `figure-energy.png`, and the generated PDF/EPUB/DOCX documents are project-owned TextbookLens test material licensed under Apache-2.0.
- `NotoSansSC-fixture-subset.otf` is a glyph subset of Noto Sans CJK SC Regular 2.004 and remains licensed under the SIL Open Font License 1.1. The complete, unmodified license is committed as `source/fonts/OFL.txt`.
- Official release: https://github.com/notofonts/noto-cjk/releases/tag/Sans2.004
- Source font: https://raw.githubusercontent.com/notofonts/noto-cjk/Sans2.004/Sans/OTF/SimplifiedChinese/NotoSansCJKsc-Regular.otf
- Source font SHA-256: `2c76254f6fc379fddfce0a7e84fb5385bb135d3e399294f6eeb6680d0365b74b`

## Reproduction

Run `npm run fixtures:build`, followed by `npm run fixtures:verify`. The committed font subset is the only prebuilt input; it was made once from the source above with FontTools 4.59.0 using only the shared fixture text glyphs.

## Committed SHA-256 values

| Path                                                  | SHA-256                                                            |
| ----------------------------------------------------- | ------------------------------------------------------------------ |
| `fixtures/source/textbook-content.json`               | `c3fe9845f9182edd3c6ae806243475d3397dac14a86acda7710c41e3231b9bdd` |
| `fixtures/source/fonts/NotoSansSC-fixture-subset.otf` | `6a389db7817debe27d902acc5e28d701f87681456f331aaf172ad9b14604f33c` |
| `fixtures/source/fonts/OFL.txt`                       | `6a73f9541c2de74158c0e7cf6b0a58ef774f5a780bf191f2d7ec9cc53efe2bf2` |
| `fixtures/source/figure-energy.png`                   | `f1281611a392673392e6d56698a2c82148754e83af7bfe88b830fed938f11e95` |
| `fixtures/textbook.pdf`                               | `9c054676cf6090d6dfb8ad38690e883103df0bb95eb67819bcb2ba7352b5518e` |
| `fixtures/textbook.epub`                              | `67fa01aecd47cdf47b8faa34b7f31e51d280d43fea14de66cf817166cbd3c94d` |
| `fixtures/textbook.docx`                              | `188dfe5630166b767bb3dbb2ea9329447751f0c47b087adca140d8ad1c84a238` |
| `fixtures/source/scanned-textbook.pdf`                | `8b4d3d6e8bf08d5f31c4ae150f1ec85b407921ed6e4470b40f72c5ce091732ee` |
| `fixtures/source/mixed-quality-textbook.pdf`          | `e707ffa8de016d56d5d334dcd75d7e14dcf198e5507e344f77391626de71b527` |

## Vision operation fixtures

| Path                                     | Format / dimensions | SHA-256                                                            |
| ---------------------------------------- | ------------------- | ------------------------------------------------------------------ |
| `fixtures/source/vision/tiny-blue.png`   | PNG / 2×2           | `daec3255de7a4747c9771880cc860ec451f948cc94c2596acfa5d79d577d704e` |
| `fixtures/source/vision/tiny-orange.jpg` | JPEG / 2×2          | `ef6aee1291db2afa2be3930c58ff8277044c3d0035cc11a25e276a3ce7da405a` |
