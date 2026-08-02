export const fixtureContent = {
  title: 'TextbookLens 测试教材',
  author: 'TextbookLens Contributors',
  firstChapter: '第一章 线性关系',
  secondChapter: '第二章 能量',
  equation: 'E = mc²',
  caption: '图 2-1：自制能量关系示意图',
} as const;

export const fixtureUrls = {
  pdf: new URL('../../fixtures/textbook.pdf', import.meta.url),
  epub: new URL('../../fixtures/textbook.epub', import.meta.url),
  docx: new URL('../../fixtures/textbook.docx', import.meta.url),
} as const;
