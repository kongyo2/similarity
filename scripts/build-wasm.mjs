import { execa } from 'execa';

await execa('wasm-pack', ['build', 'native/similarity-wasm', '--target', 'nodejs', '--release'], {
  stdio: 'inherit',
});
