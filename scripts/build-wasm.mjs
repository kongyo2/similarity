import { execa } from 'execa';
import { rm } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const scriptDir = dirname(fileURLToPath(import.meta.url));
const repoRoot = join(scriptDir, '..');
const wasmOutDir = join(repoRoot, 'dist', 'wasm');

await execa(
  'wasm-pack',
  [
    'build',
    'native/similarity-wasm',
    '--target',
    'nodejs',
    '--release',
    '--out-dir',
    '../../dist/wasm',
  ],
  { stdio: 'inherit', cwd: repoRoot },
);

// wasm-pack writes a `.gitignore` containing `*` which would cause `npm
// publish` to exclude the generated artifacts. Remove it so the WASM files
// ship inside the published tarball.
await rm(join(wasmOutDir, '.gitignore'), { force: true });
