export function isReaderFullscreen(element: HTMLElement | null): boolean {
  return element !== null && document.fullscreenElement === element;
}

export async function toggleReaderFullscreen(
  element: HTMLElement | null,
): Promise<void> {
  if (!element) return;
  if (document.fullscreenElement) {
    await document.exitFullscreen?.();
    return;
  }
  await element.requestFullscreen?.();
}
