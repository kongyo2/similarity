import { defineConfig } from "tsup";

export default defineConfig([
  {
    entry: ["src/index.ts"],
    format: ["esm"],
    target: "node20",
    dts: true,
    sourcemap: true,
    // Everything in `dist` but the WebAssembly engine, which
    // `npm run build:wasm` puts there and this build does not produce: a
    // plain `clean: true` left a working tree without an engine after
    // every `npm run build`.
    clean: ["!wasm/**"],
    splitting: false,
    outDir: "dist",
  },
  {
    entry: ["src/cli.ts"],
    format: ["esm"],
    target: "node20",
    dts: false,
    sourcemap: true,
    clean: false,
    splitting: false,
    outDir: "dist",
  },
]);
