import { Buffer } from 'node:buffer';
import { createHash } from 'node:crypto';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import fontkit from '@pdf-lib/fontkit';
import {
  AlignmentType,
  Document,
  HeadingLevel,
  ImageRun,
  Packer,
  Paragraph,
  Table,
  TableCell,
  TableRow,
  TextRun,
  WidthType,
} from 'docx';
import JSZip from 'jszip';
import { PDFDocument, rgb } from 'pdf-lib';
import { PNG } from 'pngjs';
import { format as formatWithPrettier } from 'prettier';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const baselineDate = new Date('2026-08-01T00:00:00.000Z');
const baselineIso = baselineDate.toISOString();
const fullFontSha256 =
  '2c76254f6fc379fddfce0a7e84fb5385bb135d3e399294f6eeb6680d0365b74b';
const oflSha256 =
  '6a73f9541c2de74158c0e7cf6b0a58ef774f5a780bf191f2d7ec9cc53efe2bf2';

const sourcePath = 'fixtures/source/textbook-content.json';
const fontPath = 'fixtures/source/fonts/NotoSansSC-fixture-subset.otf';
const oflPath = 'fixtures/source/fonts/OFL.txt';
const imagePath = 'fixtures/source/figure-energy.png';
const pdfPath = 'fixtures/textbook.pdf';
const epubPath = 'fixtures/textbook.epub';
const docxPath = 'fixtures/textbook.docx';
const readmePath = 'fixtures/README.md';

const hashedFiles = [
  sourcePath,
  fontPath,
  oflPath,
  imagePath,
  pdfPath,
  epubPath,
  docxPath,
];
const fixtureFiles = [...hashedFiles, readmePath];

function absolute(relativePath) {
  return path.join(root, relativePath);
}

async function readRequired(relativePath) {
  try {
    const bytes = await readFile(absolute(relativePath));
    if (bytes.length === 0) throw new Error('is empty');
    return bytes;
  } catch (error) {
    throw new Error(
      `${relativePath}: ${error.code === 'ENOENT' ? 'missing' : error.message}`,
      { cause: error },
    );
  }
}

function sha256(bytes) {
  return createHash('sha256').update(bytes).digest('hex');
}

function hasPrefix(bytes, prefix) {
  return prefix.every((value, index) => bytes[index] === value);
}

function escapeXml(value) {
  return value
    .replaceAll('&', '&amp;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;')
    .replaceAll('"', '&quot;');
}

function assertSource(content) {
  if (
    content.title !== 'TextbookLens 测试教材' ||
    content.author !== 'TextbookLens Contributors' ||
    content.language !== 'zh-CN' ||
    content.sections.length !== 2 ||
    content.sections[0].list.length !== 2 ||
    content.sections[0].table.length !== 3 ||
    content.sections[1].paragraphs[0] !== '质能关系可以写为 E = mc²。' ||
    content.sections[1].caption !== '图 2-1：自制能量关系示意图'
  ) {
    throw new Error(
      `${sourcePath}: semantic fixture contract does not match the Phase 2 plan`,
    );
  }
}

function createEnergyPng() {
  const png = new PNG({ width: 320, height: 160, colorType: 6 });
  const setPixel = (x, y, [red, green, blue, alpha = 255]) => {
    if (x < 0 || y < 0 || x >= png.width || y >= png.height) return;
    const offset = (png.width * y + x) << 2;
    png.data[offset] = red;
    png.data[offset + 1] = green;
    png.data[offset + 2] = blue;
    png.data[offset + 3] = alpha;
  };
  const fillRect = (x, y, width, height, color) => {
    for (let row = y; row < y + height; row += 1) {
      for (let column = x; column < x + width; column += 1)
        setPixel(column, row, color);
    }
  };

  fillRect(0, 0, png.width, png.height, [255, 255, 255]);
  fillRect(30, 50, 70, 60, [37, 99, 235]);
  fillRect(220, 35, 70, 90, [245, 158, 11]);
  fillRect(105, 76, 110, 8, [31, 41, 55]);
  for (let step = 0; step < 24; step += 1) {
    fillRect(191 + step, 64 + step, 3, 3, [31, 41, 55]);
    fillRect(191 + step, 94 - step, 3, 3, [31, 41, 55]);
  }
  return PNG.sync.write(png, {
    colorType: 6,
    inputColorType: 6,
    bitDepth: 8,
    deflateLevel: 9,
  });
}

function drawWrappedText(page, font, text, options) {
  const {
    x,
    y,
    size,
    maxWidth,
    lineHeight = size * 1.5,
    color = rgb(0.1, 0.12, 0.16),
  } = options;
  const lines = [];
  let current = '';
  for (const character of text) {
    const candidate = current + character;
    if (current && font.widthOfTextAtSize(candidate, size) > maxWidth) {
      const wordBoundary = current.lastIndexOf(' ');
      if (wordBoundary > 0) {
        lines.push(current.slice(0, wordBoundary));
        current = current.slice(wordBoundary + 1) + character;
      } else {
        lines.push(current);
        current = character;
      }
    } else {
      current = candidate;
    }
  }
  if (current) lines.push(current);
  lines.forEach((line, index) =>
    page.drawText(line, { x, y: y - index * lineHeight, size, font, color }),
  );
  return y - lines.length * lineHeight;
}

async function createPdf(content, fontBytes, imageBytes) {
  const document = await PDFDocument.create({ updateMetadata: false });
  document.registerFontkit(fontkit);
  document.setTitle(content.title);
  document.setAuthor(content.author);
  document.setCreator('TextbookLens fixture generator');
  document.setProducer('TextbookLens fixture generator');
  document.setCreationDate(baselineDate);
  document.setModificationDate(baselineDate);
  const font = await document.embedFont(fontBytes, { subset: false });
  const image = await document.embedPng(imageBytes);

  for (const [sectionIndex, section] of content.sections.entries()) {
    const page = document.addPage([595.28, 841.89]);
    let y = 790;
    page.drawText(content.title, {
      x: 48,
      y,
      size: 20,
      font,
      color: rgb(0.08, 0.2, 0.42),
    });
    y -= 42;
    page.drawText(section.title, {
      x: 48,
      y,
      size: 17,
      font,
      color: rgb(0.08, 0.12, 0.2),
    });
    y -= 32;
    for (const paragraph of section.paragraphs) {
      y = drawWrappedText(page, font, paragraph, {
        x: 48,
        y,
        size: 12,
        maxWidth: 499,
      });
      y -= 12;
    }

    if (section.list) {
      for (const item of section.list) {
        page.drawText(`• ${item}`, { x: 62, y, size: 12, font });
        y -= 22;
      }
    }

    if (section.table) {
      const left = 48;
      const top = y - 4;
      const cellWidth = 90;
      const cellHeight = 26;
      for (const [rowIndex, row] of section.table.entries()) {
        for (const [columnIndex, cell] of row.entries()) {
          const cellX = left + columnIndex * cellWidth;
          const cellY = top - (rowIndex + 1) * cellHeight;
          page.drawRectangle({
            x: cellX,
            y: cellY,
            width: cellWidth,
            height: cellHeight,
            borderWidth: 0.75,
            borderColor: rgb(0.25, 0.3, 0.38),
            color: rowIndex === 0 ? rgb(0.9, 0.94, 1) : rgb(1, 1, 1),
          });
          page.drawText(cell, { x: cellX + 8, y: cellY + 8, size: 11, font });
        }
      }
    }

    if (sectionIndex === 1) {
      page.drawImage(image, { x: 138, y: 350, width: 320, height: 160 });
      page.drawText(section.caption, { x: 185, y: 326, size: 11, font });
    }
  }

  return Buffer.from(
    await document.save({ useObjectStreams: false, addDefaultPage: false }),
  );
}

function paragraph(text, options = {}) {
  return new Paragraph({
    children: [new TextRun({ text, font: 'Noto Sans CJK SC' })],
    ...options,
  });
}

async function normalizeDocxZip(buffer) {
  const input = await JSZip.loadAsync(buffer);
  const output = new JSZip();
  const names = Object.keys(input.files).sort((left, right) =>
    left.localeCompare(right),
  );
  for (const name of names) {
    const entry = input.files[name];
    if (entry.dir) continue;
    let data = await entry.async('nodebuffer');
    if (name === 'docProps/core.xml') {
      data = Buffer.from(
        data
          .toString('utf8')
          .replace(
            /<dcterms:created[^>]*>.*?<\/dcterms:created>/u,
            `<dcterms:created xsi:type="dcterms:W3CDTF">${baselineIso}</dcterms:created>`,
          )
          .replace(
            /<dcterms:modified[^>]*>.*?<\/dcterms:modified>/u,
            `<dcterms:modified xsi:type="dcterms:W3CDTF">${baselineIso}</dcterms:modified>`,
          ),
      );
    }
    output.file(name, data, { date: baselineDate, createFolders: false });
  }
  return output.generateAsync({
    type: 'nodebuffer',
    compression: 'DEFLATE',
    compressionOptions: { level: 9 },
    platform: 'UNIX',
  });
}

async function createDocx(content, imageBytes) {
  const [first, second] = content.sections;
  const table = new Table({
    width: { size: 60, type: WidthType.PERCENTAGE },
    rows: first.table.map(
      (row) =>
        new TableRow({
          children: row.map(
            (cell) => new TableCell({ children: [paragraph(cell)] }),
          ),
        }),
    ),
  });
  const image = new ImageRun({
    type: 'png',
    data: imageBytes,
    transformation: { width: 320, height: 160 },
    altText: {
      title: 'Energy relation diagram',
      description: second.caption,
      name: 'figure-energy.png',
    },
  });
  const document = new Document({
    title: content.title,
    subject: 'Deterministic three-format import fixture',
    creator: content.author,
    lastModifiedBy: content.author,
    revision: 1,
    sections: [
      {
        children: [
          paragraph(content.title, { heading: HeadingLevel.TITLE }),
          paragraph(first.title, { heading: HeadingLevel.HEADING_1 }),
          ...first.paragraphs.map((text) => paragraph(text)),
          ...first.list.map((text) =>
            paragraph(text, { bullet: { level: 0 } }),
          ),
          table,
          paragraph(second.title, {
            heading: HeadingLevel.HEADING_1,
            pageBreakBefore: true,
          }),
          ...second.paragraphs.map((text) => paragraph(text)),
          new Paragraph({ children: [image], alignment: AlignmentType.CENTER }),
          paragraph(second.caption, {
            alignment: AlignmentType.CENTER,
            style: 'Caption',
          }),
        ],
      },
    ],
  });
  return normalizeDocxZip(await Packer.toBuffer(document));
}

function chapterXhtml(content, section, index) {
  const list = section.list
    ? `<ul>${section.list.map((item) => `<li>${escapeXml(item)}</li>`).join('')}</ul>`
    : '';
  const table = section.table
    ? `<table>${section.table
        .map(
          (row) =>
            `<tr>${row.map((cell) => `<td>${escapeXml(cell)}</td>`).join('')}</tr>`,
        )
        .join('')}</table>`
    : '';
  const figure = section.caption
    ? `<figure><img src="images/figure-energy.png" alt="Energy relation diagram"/><figcaption>${escapeXml(section.caption)}</figcaption></figure>`
    : '';
  return `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE html>
<html xmlns="http://www.w3.org/1999/xhtml" xml:lang="${content.language}" lang="${content.language}">
<head><title>${escapeXml(section.title)}</title><link rel="stylesheet" type="text/css" href="styles.css"/></head>
<body><section id="chapter-${index + 1}"><h1>${escapeXml(section.title)}</h1>${section.paragraphs
    .map((text) => `<p>${escapeXml(text)}</p>`)
    .join('')}${list}${table}${figure}</section></body>
</html>`;
}

async function createEpub(content, imageBytes) {
  const zip = new JSZip();
  const add = (name, data, options = {}) =>
    zip.file(name, data, {
      date: baselineDate,
      createFolders: false,
      ...options,
    });
  add('mimetype', 'application/epub+zip', { compression: 'STORE' });
  add(
    'META-INF/container.xml',
    '<?xml version="1.0" encoding="UTF-8"?><container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>',
  );
  add(
    'OEBPS/content.opf',
    `<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="book-id" xml:lang="${content.language}">
<metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:identifier id="book-id">urn:uuid:80ff48e5-7cf6-52f3-a183-18b27bfdffdc</dc:identifier><dc:title>${escapeXml(content.title)}</dc:title><dc:creator>${escapeXml(content.author)}</dc:creator><dc:language>${content.language}</dc:language><meta property="dcterms:modified">2026-08-01T00:00:00Z</meta></metadata>
<manifest><item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/><item id="css" href="styles.css" media-type="text/css"/><item id="chapter-1" href="chapter-1.xhtml" media-type="application/xhtml+xml"/><item id="chapter-2" href="chapter-2.xhtml" media-type="application/xhtml+xml"/><item id="energy-image" href="images/figure-energy.png" media-type="image/png"/></manifest>
<spine><itemref idref="chapter-1"/><itemref idref="chapter-2"/></spine></package>`,
  );
  add(
    'OEBPS/nav.xhtml',
    `<?xml version="1.0" encoding="UTF-8"?><html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops" xml:lang="${content.language}"><head><title>目录</title></head><body><nav epub:type="toc"><h1>目录</h1><ol>${content.sections.map((section, index) => `<li><a href="chapter-${index + 1}.xhtml">${escapeXml(section.title)}</a></li>`).join('')}</ol></nav></body></html>`,
  );
  add(
    'OEBPS/styles.css',
    'body{font-family:sans-serif;line-height:1.5}table{border-collapse:collapse}td{border:1px solid #667085;padding:.25rem}figure{text-align:center}img{max-width:100%}',
  );
  content.sections.forEach((section, index) =>
    add(
      `OEBPS/chapter-${index + 1}.xhtml`,
      chapterXhtml(content, section, index),
    ),
  );
  add('OEBPS/images/figure-energy.png', imageBytes);
  return zip.generateAsync({
    type: 'nodebuffer',
    compression: 'DEFLATE',
    compressionOptions: { level: 9 },
    platform: 'UNIX',
    mimeType: 'application/epub+zip',
  });
}

async function createReadme() {
  const rows = [];
  for (const relativePath of hashedFiles) {
    rows.push(
      `| \`${relativePath}\` | \`${sha256(await readRequired(relativePath))}\` |`,
    );
  }
  return formatWithPrettier(
    `# TextbookLens import fixtures

These tiny fixtures are generated deterministically from one shared semantic source for PDF, EPUB, and DOCX import tests. The metadata baseline is \`${baselineIso}\`.

## Licensing

- \`textbook-content.json\`, \`figure-energy.png\`, and the generated PDF/EPUB/DOCX documents are project-owned TextbookLens test material licensed under Apache-2.0.
- \`NotoSansSC-fixture-subset.otf\` is a glyph subset of Noto Sans CJK SC Regular 2.004 and remains licensed under the SIL Open Font License 1.1. The complete, unmodified license is committed as \`source/fonts/OFL.txt\`.
- Official release: https://github.com/notofonts/noto-cjk/releases/tag/Sans2.004
- Source font: https://raw.githubusercontent.com/notofonts/noto-cjk/Sans2.004/Sans/OTF/SimplifiedChinese/NotoSansCJKsc-Regular.otf
- Source font SHA-256: \`${fullFontSha256}\`

## Reproduction

Run \`npm run fixtures:build\`, followed by \`npm run fixtures:verify\`. The committed font subset is the only prebuilt input; it was made once from the source above with FontTools 4.59.0 using only the shared fixture text glyphs.

## Committed SHA-256 values

| Path | SHA-256 |
|---|---|
${rows.join('\n')}
`,
    { parser: 'markdown' },
  );
}

async function buildFixtures() {
  const contentBytes = await readRequired(sourcePath);
  const content = JSON.parse(contentBytes.toString('utf8'));
  assertSource(content);
  const fontBytes = await readRequired(fontPath);
  const licenseBytes = await readRequired(oflPath);
  if (sha256(licenseBytes) !== oflSha256)
    throw new Error(`${oflPath}: expected the complete unmodified OFL text`);

  await mkdir(absolute('fixtures/source'), { recursive: true });
  const imageBytes = createEnergyPng();
  await writeFile(absolute(imagePath), imageBytes);
  await writeFile(
    absolute(pdfPath),
    await createPdf(content, fontBytes, imageBytes),
  );
  await writeFile(absolute(epubPath), await createEpub(content, imageBytes));
  await writeFile(absolute(docxPath), await createDocx(content, imageBytes));
  await writeFile(absolute(readmePath), await createReadme(), 'utf8');
}

function firstZipEntry(bytes) {
  const compression = bytes.readUInt16LE(8);
  const nameLength = bytes.readUInt16LE(26);
  const extraLength = bytes.readUInt16LE(28);
  return {
    compression,
    name: bytes.subarray(30, 30 + nameLength).toString('utf8'),
    dataOffset: 30 + nameLength + extraLength,
  };
}

async function verifyZip(bytes, expectedEntries, label) {
  const zip = await JSZip.loadAsync(bytes, { checkCRC32: true });
  for (const entry of expectedEntries) {
    if (!zip.file(entry))
      throw new Error(`${label}: missing container entry ${entry}`);
  }
  return zip;
}

async function verifyFixtures() {
  const files = new Map();
  const failures = [];
  for (const relativePath of fixtureFiles) {
    try {
      files.set(relativePath, await readRequired(relativePath));
    } catch (error) {
      failures.push(error.message);
    }
  }
  if (failures.length > 0)
    throw new Error(`fixture verification failed:\n- ${failures.join('\n- ')}`);

  const signatureChecks = [
    [fontPath, [0x4f, 0x54, 0x54, 0x4f]],
    [imagePath, [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]],
    [pdfPath, [0x25, 0x50, 0x44, 0x46, 0x2d]],
    [epubPath, [0x50, 0x4b, 0x03, 0x04]],
    [docxPath, [0x50, 0x4b, 0x03, 0x04]],
  ];
  for (const [relativePath, signature] of signatureChecks) {
    if (!hasPrefix(files.get(relativePath), signature))
      failures.push(`${relativePath}: unexpected signature`);
  }

  try {
    const content = JSON.parse(files.get(sourcePath).toString('utf8'));
    assertSource(content);
    const pdf = await PDFDocument.load(files.get(pdfPath), {
      updateMetadata: false,
    });
    if (
      pdf.getPageCount() !== 2 ||
      pdf.getTitle() !== content.title ||
      pdf.getAuthor() !== content.author
    ) {
      failures.push(
        `${pdfPath}: expected two pages and shared title/author metadata`,
      );
    }

    const epubFirst = firstZipEntry(files.get(epubPath));
    if (epubFirst.name !== 'mimetype' || epubFirst.compression !== 0) {
      failures.push(
        `${epubPath}: mimetype must be the first uncompressed entry`,
      );
    }
    const epub = await verifyZip(
      files.get(epubPath),
      [
        'mimetype',
        'META-INF/container.xml',
        'OEBPS/content.opf',
        'OEBPS/nav.xhtml',
        'OEBPS/chapter-1.xhtml',
        'OEBPS/chapter-2.xhtml',
        'OEBPS/styles.css',
        'OEBPS/images/figure-energy.png',
      ],
      epubPath,
    );
    const epubText = `${await epub.file('OEBPS/chapter-1.xhtml').async('string')}\n${await epub.file('OEBPS/chapter-2.xhtml').async('string')}`;
    for (const phrase of [
      '第一章 线性关系',
      'A linear relation has a constant rate of change.',
      '第二章 能量',
      'E = mc²',
      '图 2-1：自制能量关系示意图',
    ]) {
      if (!epubText.includes(phrase))
        failures.push(`${epubPath}: missing semantic text ${phrase}`);
    }

    const docx = await verifyZip(
      files.get(docxPath),
      [
        '[Content_Types].xml',
        '_rels/.rels',
        'word/document.xml',
        'word/styles.xml',
      ],
      docxPath,
    );
    const documentXml = await docx.file('word/document.xml').async('string');
    for (const phrase of [
      '第一章 线性关系',
      '第二章 能量',
      'E = mc²',
      '图 2-1：自制能量关系示意图',
    ]) {
      if (!documentXml.includes(phrase))
        failures.push(`${docxPath}: missing semantic text ${phrase}`);
    }
    if (
      !Object.keys(docx.files).some(
        (name) => name.startsWith('word/media/') && !docx.files[name].dir,
      )
    ) {
      failures.push(`${docxPath}: missing embedded image`);
    }
  } catch (error) {
    failures.push(error.message);
  }

  if (sha256(files.get(oflPath)) !== oflSha256)
    failures.push(`${oflPath}: not the complete unmodified OFL text`);
  const readme = files.get(readmePath).toString('utf8');
  for (const marker of [
    'Apache-2.0',
    'SIL Open Font License 1.1',
    'Sans2.004',
    fullFontSha256,
  ]) {
    if (!readme.includes(marker))
      failures.push(`${readmePath}: missing license/source marker ${marker}`);
  }
  const declaredHashes = new Map(
    [
      ...readme.matchAll(/^\|\s*`([^`]+)`\s*\|\s*`([a-f0-9]{64})`\s*\|$/gmu),
    ].map((match) => [match[1], match[2]]),
  );
  for (const relativePath of hashedFiles) {
    const actual = sha256(files.get(relativePath));
    if (declaredHashes.get(relativePath) !== actual)
      failures.push(`${relativePath}: SHA-256 does not match ${readmePath}`);
  }

  if (failures.length > 0)
    throw new Error(`fixture verification failed:\n- ${failures.join('\n- ')}`);
}

if (process.argv.includes('--verify')) {
  await verifyFixtures();
  console.log('Fixture verification passed.');
} else {
  await buildFixtures();
  console.log('Fixture generation completed.');
}
