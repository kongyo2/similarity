import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { describe, expect, it } from "vitest";
import { analyzeProject } from "../src/index.js";

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
});
