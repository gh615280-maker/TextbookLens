import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const projectRoot = fileURLToPath(new URL('..', import.meta.url));
const cargoManifest = 'src-tauri/Cargo.toml';

function run(command, args) {
  const result = spawnSync(command, args, {
    cwd: projectRoot,
    stdio: 'inherit',
  });

  if (result.error) {
    throw result.error;
  }
  if (result.status !== 0) {
    process.exit(result.status ?? 1);
  }
}

run('cargo', [
  'test',
  '--manifest-path',
  cargoManifest,
  '--test',
  'bindings',
  'export_bindings',
  '--',
  '--nocapture',
]);
run('git', ['diff', '--exit-code', '--', 'src/lib/generated']);
