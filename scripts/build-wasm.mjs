// Builds the WebAssembly engine into `dist/wasm`, where the TypeScript
// layer looks for it (see `src/engine/wasm.ts`).
import { spawnSync } from "node:child_process";
import { rm } from "node:fs/promises";
import { createRequire } from "node:module";
import { dirname, join } from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";

const repoRoot = join(dirname(fileURLToPath(import.meta.url)), "..");
const wasmOutDir = join(repoRoot, "dist", "wasm");

const build = spawnSync(
  "wasm-pack",
  [
    "build",
    "native/similarity-wasm",
    "--target",
    "nodejs",
    "--release",
    // Relative to the crate, not to the working directory.
    "--out-dir",
    "../../dist/wasm",
  ],
  { stdio: "inherit", cwd: repoRoot },
);

// A command that cannot be started is an `error`, not an exit status.
if (build.error) {
  console.error(
    build.error.code === "ENOENT"
      ? "wasm-pack was not found on the PATH. Install it with `cargo install wasm-pack` " +
          "(or from https://rustwasm.github.io/wasm-pack/installer/) and add the target " +
          "with `rustup target add wasm32-unknown-unknown`."
      : `wasm-pack could not be started: ${build.error.message}`,
  );
  process.exit(1);
}
if (build.status !== 0) {
  process.exit(build.status ?? 1);
}

// wasm-pack writes a `.gitignore` containing `*`, which makes `npm publish`
// leave the generated files out of the tarball. The `package.json` it
// writes stays: it is what marks the glue module in this directory as
// CommonJS inside an ES-module package.
await rm(join(wasmOutDir, ".gitignore"), { force: true });

// A build that does not load is no build: fail here rather than in the
// first analysis someone runs.
const engine = createRequire(import.meta.url)(join(wasmOutDir, "similarity_wasm.js"));
for (const entryPoint of ["analyze_project", "check_source"]) {
  if (typeof engine[entryPoint] !== "function") {
    console.error(`The built engine does not export \`${entryPoint}\`.`);
    process.exit(1);
  }
}
