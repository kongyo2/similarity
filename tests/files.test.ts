import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { describe, expect, it } from "vitest";
import { collectTypeScriptFiles } from "../src/utils/files.js";

async function withTempProject(run: (projectDir: string) => Promise<void>) {
  const tempRoot = await fs.mkdtemp(path.join(os.tmpdir(), "similarity-ts-files-"));
  try {
    await run(tempRoot);
  } finally {
    await fs.rm(tempRoot, { recursive: true, force: true });
  }
}

describe("collectTypeScriptFiles", () => {
  it("includes explicitly passed supported files that are not ignored", async () => {
    await withTempProject(async (projectDir) => {
      await fs.mkdir(path.join(projectDir, "src"), { recursive: true });
      const filePath = path.join(projectDir, "src", "direct.ts");
      await fs.writeFile(filePath, "export const direct = true;\n", "utf8");

      const result = await collectTypeScriptFiles({
        cwd: projectDir,
        paths: ["src/direct.ts"],
        extensions: ["ts"],
        exclude: [],
      });

      expect(result.files).toEqual([filePath]);
      expect(result.skipped).toEqual([]);
      expect(result.warnings).toEqual([]);
    });
  });

  it("reuses global ignore matchers across multiple explicit files in the same base", async () => {
    await withTempProject(async (projectDir) => {
      const firstPath = path.join(projectDir, "first.ts");
      const secondPath = path.join(projectDir, "second.ts");
      await fs.writeFile(firstPath, "export const first = true;\n", "utf8");
      await fs.writeFile(secondPath, "export const second = true;\n", "utf8");

      const result = await collectTypeScriptFiles({
        cwd: projectDir,
        paths: ["first.ts", "second.ts"],
        extensions: ["ts"],
        exclude: [],
      });

      expect(result.files).toEqual([firstPath, secondPath]);
      expect(result.skipped).toEqual([]);
      expect(result.warnings).toEqual([]);
    });
  });

  it("reports unsupported path types as warnings", async () => {
    await withTempProject(async (projectDir) => {
      const devicePath = path.join(projectDir, "device.ts");
      await fs.symlink("/dev/null", devicePath);

      const result = await collectTypeScriptFiles({
        cwd: projectDir,
        paths: ["device.ts"],
        extensions: ["ts"],
        exclude: [],
      });

      expect(result.files).toEqual([]);
      expect(result.skipped).toEqual([]);
      expect(result.warnings).toEqual(["Unsupported path type: device.ts"]);
    });
  });

  it("skips explicitly passed files with disabled extensions", async () => {
    await withTempProject(async (projectDir) => {
      const filePath = path.join(projectDir, "plain.js");
      await fs.writeFile(filePath, "export const plain = true;\n", "utf8");

      const result = await collectTypeScriptFiles({
        cwd: projectDir,
        paths: ["plain.js"],
        extensions: ["ts"],
        exclude: [],
      });

      expect(result.files).toEqual([]);
      expect(result.skipped).toEqual([filePath]);
      expect(result.warnings).toEqual([]);
    });
  });

  it("falls back to the target directory when no enclosing git root is found", async () => {
    await withTempProject(async (projectDir) => {
      const cwd = path.join(projectDir, "cwd");
      await fs.mkdir(cwd);

      let externalTarget = path.join(projectDir, "external");
      for (let index = 0; index < 130; index += 1) {
        externalTarget = path.join(externalTarget, "d");
      }
      await fs.mkdir(externalTarget, { recursive: true });
      const filePath = path.join(externalTarget, "deep.ts");
      await fs.writeFile(filePath, "export const deep = true;\n", "utf8");

      const result = await collectTypeScriptFiles({
        cwd,
        paths: [externalTarget],
        extensions: ["ts"],
        exclude: [],
      });

      expect(result.files).toEqual([filePath]);
      expect(result.skipped).toEqual([]);
      expect(result.warnings).toEqual([]);
    });
  });

  it("returns no directory matches when no extensions are enabled", async () => {
    await withTempProject(async (projectDir) => {
      await fs.mkdir(path.join(projectDir, "src"), { recursive: true });
      await fs.writeFile(
        path.join(projectDir, "src", "ignored.ts"),
        "export const ignored = true;\n",
        "utf8",
      );

      const result = await collectTypeScriptFiles({
        cwd: projectDir,
        paths: ["src"],
        extensions: [],
        exclude: [],
      });

      expect(result.files).toEqual([]);
      expect(result.skipped).toEqual([]);
      expect(result.warnings).toEqual([]);
    });
  });

  it("applies explicit exclude patterns in addition to default ignores", async () => {
    await withTempProject(async (projectDir) => {
      await fs.mkdir(path.join(projectDir, "src"), { recursive: true });
      const keptPath = path.join(projectDir, "src", "kept.ts");
      const skippedPath = path.join(projectDir, "src", "skip.generated.ts");
      await fs.writeFile(keptPath, "export const kept = true;\n", "utf8");
      await fs.writeFile(skippedPath, "export const skipped = true;\n", "utf8");

      const result = await collectTypeScriptFiles({
        cwd: projectDir,
        paths: ["src"],
        extensions: ["ts"],
        exclude: ["**/*.generated.ts"],
      });

      expect(result.files).toEqual([keptPath]);
      expect(result.skipped).toEqual([skippedPath]);
      expect(result.warnings).toEqual([]);
    });
  });
});
