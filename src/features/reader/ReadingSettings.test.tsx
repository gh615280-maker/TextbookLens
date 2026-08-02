import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';

import { ReadingSettings } from './ReadingSettings';

const settings = {
  fontScale: 1.25,
  lineHeight: 1.8,
  readerWidth: 80,
  pdfZoom: 1.5,
  theme: 'dark' as const,
};
describe('ReadingSettings', () => {
  it('uses accessible controls, shows values, restricts PDF to zoom, and restores defaults', async () => {
    const user = userEvent.setup();
    const onChange = vi.fn();
    render(
      <ReadingSettings settings={settings} format="pdf" onChange={onChange} />,
    );
    expect(screen.getByRole('slider', { name: 'PDF 缩放' })).toHaveValue('1.5');
    expect(
      screen.queryByRole('slider', { name: '字体大小' }),
    ).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: /主题/ })).toHaveTextContent(
      'dark',
    );
    await user.click(screen.getByRole('button', { name: '恢复默认' }));
    expect(onChange).toHaveBeenCalledWith({
      fontScale: 1,
      lineHeight: 1.6,
      readerWidth: 72,
      pdfZoom: 1,
      theme: 'system',
    });
  });
});
