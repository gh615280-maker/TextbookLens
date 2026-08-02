import { describe, expect, it, vi } from 'vitest';

import type { ReaderAdapter, ReaderAdapterEvents } from './contracts';
import { ReaderController } from './ReaderController';
import type { ReaderApi, ReaderBootstrap } from './api';

const bookId = '4f9a2c86-0da8-4dd4-a255-39b4cff89c66';

function bootstrap(format: ReaderBootstrap['book']['format'] = 'pdf'): ReaderBootstrap {
  return {
    book: { id: bookId, format },
    lastLocator: format === 'pdf'
      ? { format: 'pdf', startPage: 1, endPage: 1, rectsByPage: null }
      : null,
  } as ReaderBootstrap;
}

class FakeApi implements ReaderApi {
  current = bootstrap();
  readonly getReaderBootstrap = vi.fn(async () => this.current);
  readonly readBookSource = vi.fn(async () => new Uint8Array([1, 2, 3]));
  readonly readDerivedText = vi.fn(async () => '<p>safe</p>');
  readonly getReaderSettings = vi.fn();
  readonly updateReaderSettings = vi.fn();
  readonly saveReadingProgress = vi.fn();
}

function adapter(format: ReaderAdapter['format']) {
  const value: ReaderAdapter = {
    format,
    open: vi.fn(async () => {}),
    getSelectionSnapshot: () => null,
    navigate: vi.fn(),
    showAnnotations: vi.fn(),
    search: vi.fn(),
    getProgress: () => ({ fraction: 0, locator: null }),
    dispose: vi.fn(),
  };
  return value;
}

describe('ReaderController', () => {
  it('opens only the matching adapter and releases document bytes after opening', async () => {
    const api = new FakeApi();
    const pdf = adapter('pdf');
    const epub = adapter('epub');
    const controller = new ReaderController(api, { pdf: () => pdf, epub: () => epub });

    await controller.open(bookId);

    expect(pdf.open).toHaveBeenCalledWith(
      expect.objectContaining({ kind: 'document_bytes' }),
      api.current.lastLocator,
    );
    expect(epub.open).not.toHaveBeenCalled();
    expect(controller.sourceForTesting()).toBeNull();
  });

  it('uses sanitized HTML only for DOCX and disposes the previous adapter', async () => {
    const api = new FakeApi();
    const pdf = adapter('pdf');
    const docx = adapter('docx');
    const controller = new ReaderController(api, { pdf: () => pdf, docx: () => docx });

    await controller.open(bookId);
    api.current = bootstrap('docx');
    await controller.open(bookId);

    expect(pdf.dispose).toHaveBeenCalledOnce();
    expect(docx.open).toHaveBeenCalledWith({ kind: 'sanitized_html', html: '<p>safe</p>' }, null);
    expect(api.readBookSource).toHaveBeenCalledOnce();
    expect(api.readDerivedText).toHaveBeenCalledWith(bookId, 'document.html');
  });

  it('reports an unsupported format safely and dispose is idempotent', async () => {
    const api = new FakeApi();
    const onFailure = vi.fn<ReaderAdapterEvents['onFailure']>();
    const controller = new ReaderController(api, {}, { onFailure });

    await controller.open(bookId);
    controller.dispose();
    controller.dispose();

    expect(onFailure).toHaveBeenCalledWith(expect.objectContaining({ code: 'INVALID_INPUT' }));
  });
});
