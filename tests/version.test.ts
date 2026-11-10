import fs from "node:fs/promises";
import { describe, expect, it } from "vitest";

const root = new URL("../", import.meta.url);

async function read(relativePath: string): Promise<string> {
  return fs.readFile(new URL(relativePath, root), "utf8");
}

/** The `version` of the `[package]` table of a Cargo manifest. */
function cargoVersion(manifest: string): string | undefined {
  return /^\[package\][^[]*?^version = "([^"]+)"/ms.exec(manifest)?.[1];
}

// The version is written in nine places, and a release that misses one
// ships an engine that calls itself something else than its package.
describe("the version", () => {
  it("is the same wherever it is stated", async () => {
    const manifest = JSON.parse(await read("package.json")) as { version: string };
    const version = manifest.version;
    expect(version).toMatch(/^\d+\.\d+\.\d+$/);

    const lock = JSON.parse(await read("package-lock.json")) as {
      version: string;
      packages: Record<string, { version: string }>;
    };
    expect(lock.version).toBe(version);
    expect(lock.packages[""]?.version).toBe(version);

    const crates = ["similarity-core-fork", "similarity-core-ts", "similarity-wasm"];
    for (const crate of crates) {
      expect(cargoVersion(await read(`native/${crate}/Cargo.toml`)), crate).toBe(version);
    }

    // The three crates in the workspace's lock file.
    const cargoLock = await read("native/Cargo.lock");
    const locked = [...cargoLock.matchAll(/name = "(similarity-[a-z-]+)"\nversion = "([^"]+)"/g)];
    expect(locked.map((entry) => entry[1]).sort()).toEqual([
      "similarity-core",
      "similarity-core-ts",
      "similarity-wasm",
    ]);
    for (const [, crate, lockedVersion] of locked) {
      expect(lockedVersion, crate).toBe(version);
    }
  });

  it("heads the changelog", async () => {
    const manifest = JSON.parse(await read("package.json")) as { version: string };
    const newest = /^## (\d+\.\d+\.\d+) — \d{4}-\d{2}-\d{2}$/m.exec(await read("CHANGELOG.md"));
    expect(newest?.[1]).toBe(manifest.version);
  });
});
