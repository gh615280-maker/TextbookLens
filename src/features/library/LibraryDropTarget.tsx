import { getCurrentWebview } from '@tauri-apps/api/webview';
import { useEffect, useRef, useState, type ReactNode } from 'react';

const supportedExtensions = new Set(['pdf', 'epub', 'docx']);

export interface LibraryDropTargetProps {
  children: ReactNode;
  onDrop(sourcePaths: readonly string[]): void | Promise<void>;
}

/**
 * Receives only native Tauri file-drop events. Browser data-transfer payloads
 * (text, URLs, and arbitrary File objects) are intentionally never inspected.
 */
export function LibraryDropTarget({
  children,
  onDrop,
}: LibraryDropTargetProps) {
  const [isDragging, setIsDragging] = useState(false);
  const onDropRef = useRef(onDrop);
  onDropRef.current = onDrop;

  useEffect(() => {
    let disposed = false;
    let dragDepth = 0;
    let unlisten: (() => void) | undefined;

    void Promise.resolve()
      .then(() =>
        getCurrentWebview().onDragDropEvent((event) => {
          if (disposed) return;
          switch (event.payload.type) {
            case 'enter':
              dragDepth += 1;
              setIsDragging(true);
              break;
            case 'over':
              setIsDragging(true);
              break;
            case 'leave':
              dragDepth = Math.max(0, dragDepth - 1);
              if (dragDepth === 0) setIsDragging(false);
              break;
            case 'drop': {
              dragDepth = 0;
              setIsDragging(false);
              const sourcePaths = event.payload.paths.filter(
                isSupportedSourcePath,
              );
              if (sourcePaths.length > 0) void onDropRef.current(sourcePaths);
              break;
            }
          }
        }),
      )
      .then((nextUnlisten) => {
        if (disposed) nextUnlisten();
        else unlisten = nextUnlisten;
      })
      .catch(() => {
        // Drag and drop is an enhancement; picker import remains available.
      });

    return () => {
      disposed = true;
      dragDepth = 0;
      setIsDragging(false);
      unlisten?.();
    };
  }, []);

  return (
    <div className="library-drop-target">
      {children}
      {isDragging ? (
        <div
          aria-label="Drop supported textbook files to import"
          className="library-drop-target__overlay"
          role="status"
        >
          Drop PDF, EPUB, or DOCX files to import
        </div>
      ) : null}
    </div>
  );
}

export function isSupportedSourcePath(sourcePath: string): boolean {
  if (!isNativeFilesystemPath(sourcePath)) return false;
  const extension = sourcePath.split(/[\\/]/u).at(-1)?.split('.').at(-1);
  return (
    extension !== undefined && supportedExtensions.has(extension.toLowerCase())
  );
}

function isNativeFilesystemPath(value: string): boolean {
  return /^[a-z]:[\\/]/iu.test(value) || value.startsWith('\\\\');
}
