export function addDocxRangeOverlay(root: HTMLElement, range: Range): () => void {
  const overlay = Object.assign(document.createElement('div'), { className: 'docx-marker-overlay' }); overlay.setAttribute('aria-hidden', 'true');
  for (const rect of range.getClientRects()) { const marker = document.createElement('span'); Object.assign(marker.style, { left: `${rect.left - root.getBoundingClientRect().left}px`, top: `${rect.top - root.getBoundingClientRect().top}px`, width: `${rect.width}px`, height: `${rect.height}px` }); overlay.append(marker); }
  root.append(overlay); return () => overlay.remove();
}
