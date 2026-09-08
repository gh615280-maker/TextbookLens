import { cp, mkdir, readdir } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('..', import.meta.url));
const source = path.join(root, 'node_modules', 'pdfjs-dist');
const target = path.join(root, 'public', 'pdfjs');
await mkdir(target, { recursive: true });
for (const directory of ['cmaps', 'standard_fonts']) {
  await cp(path.join(source, directory), path.join(target, directory), {
    recursive: true,
  });
}
await mkdir(path.join(target, 'wasm'), { recursive: true });
for (const file of await readdir(path.join(source, 'wasm'))) {
  if (file.startsWith('quickjs')) continue;
  await cp(path.join(source, 'wasm', file), path.join(target, 'wasm', file));
}
await cp(path.join(source, 'LICENSE'), path.join(target, 'LICENSE'));
console.log('Local PDF decoder, CMap, font and license resources are ready.');
