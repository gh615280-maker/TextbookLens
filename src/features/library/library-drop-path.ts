const supportedExtensions = new Set(['pdf', 'epub', 'docx']);

/** Accepts only native Windows paths that can be handed to Rust for file validation. */
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
