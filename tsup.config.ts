import { defineConfig } from "tsup";

export default defineConfig([
  {
    entry: ["src/index.ts"],
    format: ["esm"],
    target: "node20",
    dts: true,
    sourcemap: true,
    clean: true,
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
