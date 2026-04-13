import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { describe, expect, it } from "vitest";
import { analyzeProject, resolveAnalyzeOptions } from "../src/index.js";

async function withTempProject(run: (projectDir: string) => Promise<void>) {
  const tempRoot = await fs.mkdtemp(path.join(os.tmpdir(), "similarity-ts-"));
  try {
    await run(tempRoot);
  } finally {
    await fs.rm(tempRoot, { recursive: true, force: true });
  }
}

async function createFixtureProject(projectDir: string) {
  await fs.mkdir(path.join(projectDir, "src"), { recursive: true });
  await fs.writeFile(
    path.join(projectDir, "src", "functions_a.ts"),
    `
export function calculateSum(values: number[]): number {
  let total = 0;
  let count = 0;
  for (const value of values) {
    if (value > 0) {
      total += value;
      count += 1;
    }
  }
  if (count === 0) {
    return 0;
  }
  return total;
}
`,
    "utf8",
  );
  await fs.writeFile(
    path.join(projectDir, "src", "functions_b.ts"),
    `
export function computeTotal(numbers: number[]): number {
  let total = 0;
  let count = 0;
  for (const item of numbers) {
    if (item > 0) {
      total += item;
      count += 1;
    }
  }
  if (count === 0) {
    return 0;
  }
  return total;
}
`,
    "utf8",
  );
  await fs.writeFile(
    path.join(projectDir, "src", "types.ts"),
    `
export interface User {
  id: string;
  name: string;
  active: boolean;
}

export interface Person {
  id: string;
  name: string;
  active: boolean;
}
`,
    "utf8",
  );
  await fs.writeFile(
    path.join(projectDir, "src", "classes.ts"),
    `
export class UserStore {
  constructor(private readonly items: string[]) {}
  add(name: string): void {
    this.items.push(name);
  }
}

export class PersonStore {
  constructor(private readonly items: string[]) {}
  add(name: string): void {
    this.items.push(name);
  }
}
`,
    "utf8",
  );
  await fs.writeFile(
    path.join(projectDir, "src", "overlap.ts"),
    `
export function overlapA(items: string[]): string[] {
  const output: string[] = [];
  for (const item of items) {
    output.push(item.trim().toLowerCase());
  }
  return output;
}

export function overlapB(items: string[]): string[] {
  const output: string[] = [];
  for (const item of items) {
    output.push(item.trim().toLowerCase());
  }
  return output;
}
`,
    "utf8",
  );
  await fs.writeFile(
    path.join(projectDir, "src", "ignore.js"),
    `
export const x = 1;
`,
    "utf8",
  );
}

describe("analyzeProject", () => {
  it("detects similarities across all modes", async () => {
    await withTempProject(async (projectDir) => {
      await createFixtureProject(projectDir);
      const report = await analyzeProject({
        cwd: projectDir,
        paths: ["src"],
        threshold: 0.6,
        modes: ["functions", "types", "classes", "overlap"],
        includeTypeLiterals: true,
      });

      expect(report.stats.fileCount).toBe(5);
      expect(report.byMode.functions.length).toBeGreaterThan(0);
      expect(report.byMode.types.length).toBeGreaterThan(0);
      expect(report.byMode.classes.length).toBeGreaterThan(0);
      expect(report.byMode.overlap.length).toBeGreaterThan(0);
    });
  });

  it("supports cross-file-only constraints", async () => {
    await withTempProject(async (projectDir) => {
      await createFixtureProject(projectDir);
      const report = await analyzeProject({
        cwd: projectDir,
        paths: ["src"],
        modes: ["functions"],
        threshold: 0.6,
        crossFileOnly: true,
      });

      expect(report.byMode.functions.length).toBeGreaterThan(0);
      expect(
        report.byMode.functions.every((pair) => pair.left.filePath !== pair.right.filePath),
      ).toBe(true);
    });
  });

  it("rejects non-integer numeric options via resolveAnalyzeOptions", () => {
    expect(() =>
      resolveAnalyzeOptions({ paths: ["."], minLines: Number.NaN }),
    ).toThrow("minLines must be a positive integer");
    expect(() =>
      resolveAnalyzeOptions({ paths: ["."], overlapMinWindow: 1.5 }),
    ).toThrow("overlapMinWindow must be a positive integer");
  });

  it("rejects overlapMinWindow greater than overlapMaxWindow", () => {
    expect(() =>
      resolveAnalyzeOptions({
        paths: ["."],
        overlapMinWindow: 30,
        overlapMaxWindow: 5,
      }),
    ).toThrow("overlapMinWindow must be less than or equal to overlapMaxWindow");
  });

  it("discovers files with a single extension filter", async () => {
    await withTempProject(async (projectDir) => {
      await fs.mkdir(path.join(projectDir, "src"), { recursive: true });
      await fs.writeFile(
        path.join(projectDir, "src", "only.ts"),
        "export const only = 1;\n",
        "utf8",
      );
      await fs.writeFile(
        path.join(projectDir, "src", "other.tsx"),
        "export const other = 2;\n",
        "utf8",
      );
      const report = await analyzeProject({
        cwd: projectDir,
        paths: ["src"],
        extensions: ["ts"],
      });
      expect(report.stats.fileCount).toBe(1);
      expect(report.analyzedFiles[0]?.endsWith("only.ts")).toBe(true);
    });
  });

  it("honors a .gitignore located in the target directory", async () => {
    await withTempProject(async (projectDir) => {
      const targetDir = path.join(projectDir, "app");
      await fs.mkdir(targetDir, { recursive: true });
      await fs.writeFile(path.join(targetDir, ".gitignore"), "ignored.ts\n", "utf8");
      await fs.writeFile(
        path.join(targetDir, "kept.ts"),
        "export const a = 1;\n",
        "utf8",
      );
      await fs.writeFile(
        path.join(targetDir, "ignored.ts"),
        "export const b = 2;\n",
        "utf8",
      );

      const report = await analyzeProject({
        cwd: projectDir,
        paths: ["app"],
      });
      expect(report.stats.fileCount).toBe(1);
      expect(report.analyzedFiles[0]?.endsWith("kept.ts")).toBe(true);
      expect(report.skippedFiles.some((file) => file.endsWith("ignored.ts"))).toBe(true);
    });
  });

  it("supports same-file-only constraints", async () => {
    await withTempProject(async (projectDir) => {
      await fs.mkdir(path.join(projectDir, "src"), { recursive: true });
      await fs.writeFile(
        path.join(projectDir, "src", "duplicates.ts"),
        `
export function alpha(values: number[]): number {
  let total = 0;
  let count = 0;
  for (const value of values) {
    if (value > 0) {
      total += value;
      count += 1;
    }
  }
  if (count === 0) {
    return 0;
  }
  return total;
}

export function beta(numbers: number[]): number {
  let total = 0;
  let count = 0;
  for (const item of numbers) {
    if (item > 0) {
      total += item;
      count += 1;
    }
  }
  if (count === 0) {
    return 0;
  }
  return total;
}
`,
        "utf8",
      );

      const report = await analyzeProject({
        cwd: projectDir,
        paths: ["src"],
        modes: ["functions"],
        threshold: 0.6,
        sameFileOnly: true,
      });

      expect(report.byMode.functions.length).toBeGreaterThan(0);
      expect(
        report.byMode.functions.every((pair) => pair.left.filePath === pair.right.filePath),
      ).toBe(true);
    });
  });
});
