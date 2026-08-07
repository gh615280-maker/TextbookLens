import { describe, expect, it } from 'vitest';

import {
  clientSelectionRect,
  iframeClientPointToLocal,
} from './region-preview-geometry';

describe('region preview geometry', () => {
  it('keeps both preview edges under the pointer through outer zoom', () => {
    const container = document.createElement('div');
    setLayoutSize(container, 200, 100);
    setBounds(container, 40, 70, 300, 150);

    expect(
      clientSelectionRect(container, { x: 70, y: 85 }, { x: 250, y: 175 }),
    ).toEqual({ left: 20, top: 10, width: 120, height: 60 });
  });

  it('maps iframe-local pointers through independently measured frame and reader scales', () => {
    const reader = document.createElement('div');
    const frame = document.createElement('iframe');
    setLayoutSize(reader, 400, 300);
    setLayoutSize(frame, 200, 100);
    setBounds(reader, 100, 50, 600, 450);
    setBounds(frame, 160, 95, 300, 150);

    expect(iframeClientPointToLocal(reader, frame, { x: 40, y: 20 })).toEqual({
      x: 80,
      y: 50,
    });
  });
});

function setLayoutSize(element: HTMLElement, width: number, height: number) {
  Object.defineProperties(element, {
    offsetWidth: { configurable: true, value: width },
    offsetHeight: { configurable: true, value: height },
  });
}

function setBounds(
  element: HTMLElement,
  left: number,
  top: number,
  width: number,
  height: number,
) {
  element.getBoundingClientRect = () =>
    ({
      left,
      top,
      right: left + width,
      bottom: top + height,
      width,
      height,
      x: left,
      y: top,
      toJSON: () => ({}),
    }) as DOMRect;
}
