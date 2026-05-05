import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { performance } from "node:perf_hooks";
import { describe, expect, it } from "vitest";
import { analyzeProject } from "../src/index.js";
import type { AnalyzerMode, SimilarityPair } from "../src/types.js";

async function withTempProject(run: (projectDir: string) => Promise<void>) {
  const tempRoot = await fs.mkdtemp(path.join(os.tmpdir(), "similarity-ts-accuracy-"));
  try {
    await run(tempRoot);
  } finally {
    await fs.rm(tempRoot, { recursive: true, force: true });
  }
}

async function writeSource(projectDir: string, relativePath: string, content: string) {
  const filePath = path.join(projectDir, relativePath);
  await fs.mkdir(path.dirname(filePath), { recursive: true });
  await fs.writeFile(filePath, content, "utf8");
  return filePath;
}

function hasPair(pairs: SimilarityPair[], leftName: string, rightName: string): boolean {
  return pairs.some((pair) => {
    const names = [pair.left.symbolName, pair.right.symbolName].sort();
    return names[0] === [leftName, rightName].sort()[0] &&
      names[1] === [leftName, rightName].sort()[1];
  });
}

async function analyzeTempProject(
  projectDir: string,
  modes: AnalyzerMode[],
  extra: Partial<Parameters<typeof analyzeProject>[0]> = {},
) {
  return analyzeProject({
    cwd: projectDir,
    paths: ["src"],
    modes,
    threshold: 0.7,
    ...extra,
  });
}

describe("accuracy regressions adapted from mizchi/similarity", () => {
  it("detects renamed but structurally equivalent functions within one file", async () => {
    await withTempProject(async (projectDir) => {
      await writeSource(
        projectDir,
        "src/sums.ts",
        `
export function calculateSum(numbers: number[]): number {
  if (numbers.length === 0) return 0;
  let total = 0;
  for (const number of numbers) {
    total += number;
  }
  return total;
}

export function computeTotal(values: number[]): number {
  if (values.length === 0) return 0;
  let sum = 0;
  for (const value of values) {
    sum += value;
  }
  return sum;
}
`,
      );

      const report = await analyzeTempProject(projectDir, ["functions"], {
        threshold: 0.7,
        minLines: 3,
        noSizePenalty: true,
        sameFileOnly: true,
      });

      expect(hasPair(report.byMode.functions, "calculateSum", "computeTotal")).toBe(true);
      expect(report.byMode.functions[0]?.similarity).toBeGreaterThan(0.8);
    });
  });

  it("filters moderate short-function matches at a high threshold", async () => {
    await withTempProject(async (projectDir) => {
      await writeSource(
        projectDir,
        "src/math.ts",
        `
export function add(a: number, b: number): number {
  return a + b;
}

export function sum(x: number, y: number): number {
  return x + y;
}
`,
      );

      const low = await analyzeTempProject(projectDir, ["functions"], {
        threshold: 0.5,
        minLines: 1,
        noSizePenalty: true,
      });
      const high = await analyzeTempProject(projectDir, ["functions"], {
        threshold: 0.95,
        minLines: 1,
        noSizePenalty: true,
      });

      expect(hasPair(low.byMode.functions, "add", "sum")).toBe(true);
      expect(hasPair(high.byMode.functions, "add", "sum")).toBe(false);
    });
  });

  it("keeps all members visible when three functions share the same structure", async () => {
    await withTempProject(async (projectDir) => {
      await writeSource(
        projectDir,
        "src/cluster.ts",
        `
export function calculateSum(numbers: number[]): number {
  let total = 0;
  for (const number of numbers) {
    total += number;
  }
  return total;
}

export function computeTotal(values: number[]): number {
  let total = 0;
  for (const value of values) {
    total += value;
  }
  return total;
}

export function aggregateAmount(items: number[]): number {
  let total = 0;
  for (const item of items) {
    total += item;
  }
  return total;
}
`,
      );

      const report = await analyzeTempProject(projectDir, ["functions"], {
        threshold: 0.8,
        minLines: 1,
        noSizePenalty: true,
      });
      const detectedNames = new Set(
        report.byMode.functions.flatMap((pair) => [
          pair.left.symbolName,
          pair.right.symbolName,
        ]),
      );

      expect(detectedNames).toEqual(
        new Set(["calculateSum", "computeTotal", "aggregateAmount"]),
      );
    });
  });

  it("ignores node_modules through the default exclude set", async () => {
    await withTempProject(async (projectDir) => {
      await writeSource(
        projectDir,
        "node_modules/package/ignored.ts",
        "export const ignored = true;\n",
      );
      await writeSource(projectDir, "src/app.ts", "export const found = true;\n");

      const report = await analyzeProject({
        cwd: projectDir,
        paths: ["."],
      });

      expect(report.stats.fileCount).toBe(1);
      expect(report.analyzedFiles[0]?.endsWith("src/app.ts")).toBe(true);
      expect(report.skippedFiles.some((file) =>
        file.endsWith("node_modules/package/ignored.ts"),
      )).toBe(true);
    });
  });

  it("avoids high-confidence false positives for similar loops with different intent", async () => {
    await withTempProject(async (projectDir) => {
      await writeSource(
        projectDir,
        "src/loop_intent.ts",
        `
export function findMax(numbers: number[]): number {
  let max = numbers[0];
  for (let index = 1; index < numbers.length; index++) {
    if (numbers[index] > max) {
      max = numbers[index];
    }
  }
  return max;
}

export function countOccurrences(text: string, char: string): number {
  let count = 0;
  for (let index = 0; index < text.length; index++) {
    if (text[index] === char) {
      count += 1;
    }
  }
  return count;
}
`,
      );

      const report = await analyzeTempProject(projectDir, ["functions"], {
        threshold: 0.6,
        minLines: 3,
      });

      expect(hasPair(report.byMode.functions, "findMax", "countOccurrences")).toBe(false);
    });
  });

  it("detects similar interfaces across files", async () => {
    await withTempProject(async (projectDir) => {
      await writeSource(
        projectDir,
        "src/user.ts",
        `
export interface User {
  id: number;
  name: string;
  email: string;
  createdAt: Date;
}
`,
      );
      await writeSource(
        projectDir,
        "src/person.ts",
        `
export interface Person {
  id: number;
  name: string;
  email: string;
  birthDate: Date;
}
`,
      );

      const report = await analyzeTempProject(projectDir, ["types"], {
        threshold: 0.7,
      });

      expect(hasPair(report.byMode.types, "User", "Person")).toBe(true);
    });
  });

  it("requires allowCrossKind before matching an interface to an equivalent type alias", async () => {
    await withTempProject(async (projectDir) => {
      await writeSource(
        projectDir,
        "src/mixed.ts",
        `
export interface IUser {
  id: number;
  name: string;
  email: string;
  permissions: string[];
}

export type TUser = {
  id: number;
  name: string;
  email: string;
  permissions: string[];
};
`,
      );

      const defaultReport = await analyzeTempProject(projectDir, ["types"], {
        threshold: 0.9,
      });
      const crossKindReport = await analyzeTempProject(projectDir, ["types"], {
        threshold: 0.9,
        allowCrossKind: true,
      });

      expect(hasPair(defaultReport.byMode.types, "IUser", "TUser")).toBe(false);
      expect(hasPair(crossKindReport.byMode.types, "IUser", "TUser")).toBe(true);
    });
  });

  it("does not report broad generic interfaces as similar by default", async () => {
    await withTempProject(async (projectDir) => {
      await writeSource(
        projectDir,
        "src/generics.ts",
        `
export interface Response<T> {
  data: T;
  status: number;
  message: string;
}

export interface ApiResult<T> {
  result: T;
  code: number;
  description: string;
}

export interface ServerResponse<T> {
  payload: T;
  statusCode: number;
  error?: string;
}
`,
      );

      const report = await analyzeTempProject(projectDir, ["types"], {
        threshold: 0.7,
      });

      expect(report.byMode.types).toHaveLength(0);
    });
  });

  it("does not report unrelated small object types at a high threshold", async () => {
    await withTempProject(async (projectDir) => {
      await writeSource(
        projectDir,
        "src/different.ts",
        `
export interface Point {
  x: number;
  y: number;
}

export interface User {
  id: string;
  name: string;
  email: string;
}

export interface Config {
  debug: boolean;
  timeout: number;
  retryCount: number;
}
`,
      );

      const report = await analyzeTempProject(projectDir, ["types"], {
        threshold: 0.9,
      });

      expect(report.byMode.types).toHaveLength(0);
    });
  });

  it("honors similarity-ignore comments for functions and types", async () => {
    await withTempProject(async (projectDir) => {
      await writeSource(
        projectDir,
        "src/ignored.ts",
        `
export function calculateTotal(items: number[]): number {
  let total = 0;
  for (const item of items) {
    total += item;
  }
  return total;
}

// similarity-ignore
export function computeTotal(values: number[]): number {
  let total = 0;
  for (const value of values) {
    total += value;
  }
  return total;
}

export interface User {
  id: string;
  email: string;
}

// similarity-ignore
export interface IgnoredUser {
  id: string;
  email: string;
}
`,
      );

      const report = await analyzeTempProject(projectDir, ["functions", "types"], {
        threshold: 0.6,
        noSizePenalty: true,
      });

      expect(hasPair(report.byMode.functions, "calculateTotal", "computeTotal")).toBe(false);
      expect(hasPair(report.byMode.types, "User", "IgnoredUser")).toBe(false);
    });
  });

  it("includes TSX files in the default extension set", async () => {
    await withTempProject(async (projectDir) => {
      await writeSource(
        projectDir,
        "src/component.tsx",
        `
interface ButtonProps {
  label: string;
  onClick: () => void;
}

export function Button({ label, onClick }: ButtonProps) {
  return React.createElement("button", { onClick }, label);
}

export function PrimaryButton({ label, onClick }: ButtonProps) {
  return React.createElement("button", { onClick, className: "primary" }, label);
}
`,
      );

      const report = await analyzeTempProject(projectDir, ["functions"], {
        threshold: 0.5,
        minLines: 1,
        noSizePenalty: true,
      });

      expect(report.stats.fileCount).toBe(1);
      expect(hasPair(report.byMode.functions, "Button", "PrimaryButton")).toBe(true);
    });
  });

  it("detects exact duplicated validation blocks in overlap mode", async () => {
    await withTempProject(async (projectDir) => {
      await writeSource(
        projectDir,
        "src/validation.ts",
        `
export function validateUser(user: { email: string }) {
  if (!user.email) {
    throw new Error("Email is required");
  }
  if (!user.email.includes("@")) {
    throw new Error("Invalid email format");
  }
  if (user.email.length > 100) {
    throw new Error("Email too long");
  }
  return user.email;
}

export function validateAdmin(admin: { email: string }) {
  if (!admin.email) {
    throw new Error("Email is required");
  }
  if (!admin.email.includes("@")) {
    throw new Error("Invalid email format");
  }
  if (admin.email.length > 100) {
    throw new Error("Email too long");
  }
  return admin.email;
}
`,
      );

      const report = await analyzeTempProject(projectDir, ["overlap"], {
        threshold: 0.5,
        overlapMinWindow: 3,
        overlapMaxWindow: 25,
        overlapSizeTolerance: 0.5,
      });

      expect(report.byMode.overlap.length).toBeGreaterThan(0);
      expect(report.byMode.overlap.some((pair) => pair.similarity > 0.9)).toBe(true);
    });
  });

  it("keeps type fingerprint analysis practical on many related declarations", async () => {
    await withTempProject(async (projectDir) => {
      const declarations: string[] = [];
      for (let index = 0; index < 50; index += 1) {
        declarations.push(`
export interface Type${index} {
  field1: string;
  field2: number;
  field3: boolean;
  nested: { value: number };
  field${index}: unknown;
}
`);
      }
      await writeSource(projectDir, "src/many_types.ts", declarations.join("\n"));

      const start = performance.now();
      const report = await analyzeTempProject(projectDir, ["types"], {
        threshold: 0.95,
      });
      const elapsedMs = performance.now() - start;

      expect(report.stats.fileCount).toBe(1);
      expect(elapsedMs).toBeLessThan(5_000);
    });
  }, 10_000);
});
